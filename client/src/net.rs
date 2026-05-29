use std::sync::Mutex;

use bevy::prelude::*;
use common::broker_messages::BrokerMessage;
use common::packets::{PositionBatch, PositionSnapshot};
use common::topics::{
    deserialize_shard_snapshot_payload, serialize_position_payload, PositionPayload,
    ShardSnapshotPayload, Topic,
};
use common::Vec2;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream, GameStreamReliability};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

use super::{GameSession, GameState};

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PositionBatchReceived>()
            .add_systems(OnEnter(GameState::Connecting), start_connect)
            .add_systems(
                Update,
                (poll_net_events, tick_connect_timeout).run_if(in_state(GameState::Connecting)),
            )
            .add_systems(Update, receive_packets.run_if(in_state(GameState::InGame)));
    }
}

// ---------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
struct ShardSnapshotPlayer {
    id: Uuid,
    entity_id: u32,
    username: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
struct ShardSnapshotState {
    players: Vec<ShardSnapshotPlayer>,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Wraps the active `GamePeer` in a Mutex so it satisfies Bevy's Sync bound.
#[derive(Resource)]
pub struct ActivePeer(pub Mutex<GamePeer>);

/// Countdown started when we enter Connecting. If no `Connected` event arrives
/// before it expires the client returns to Login.
#[derive(Resource)]
struct ConnectTimeout(Timer);

/// The UUID assigned by the server in the `Welcome` message.
/// Also stored as a hash-derived `u32` for the position-interpolation system.
#[derive(Resource, Clone, Copy)]
pub struct MyEntityId(pub u32);

/// The active broker connection handle. Inserted once the QUIC handshake
/// completes. Used by other systems (e.g. input) to send datagrams.
#[derive(Resource, Clone, Copy)]
pub struct BrokerConn(pub GameConnection);

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

fn start_connect(mut commands: Commands, session: Res<GameSession>) {
    tracing::info!(
        "Connecting to game broker at {}:{}",
        session.broker_ip,
        session.broker_port
    );

    let peer = GamePeer::new(QuicBackend::new());
    if let Err(e) = peer.connect(&session.broker_ip, session.broker_port) {
        tracing::error!("Failed to initiate QUIC connection: {e:?}");
    }

    commands.insert_resource(ActivePeer(Mutex::new(peer)));
    commands.insert_resource(ConnectTimeout(Timer::from_seconds(10.0, TimerMode::Once)));
}
fn poll_net_events(
    mut commands: Commands,
    session: Res<GameSession>,
    peer_res: Option<ResMut<ActivePeer>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else { return };

    loop {
        let event = match peer.poll() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => {
                tracing::error!("GamePeer backend thread crashed — returning to login");
                next_state.set(GameState::Login);
                break;
            }
        };
        match event {
            GameNetworkEvent::Connected(conn) => {
                tracing::info!("QUIC connected (id={:?}); opening streams...", conn.connection_id);
                commands.insert_resource(BrokerConn(conn));

                if let Err(e) = peer.create_stream(conn, GameStreamReliability::Reliable) {
                    tracing::error!("Failed to initiate reliable stream: {e:?}");
                }
                if let Err(e) = peer.create_stream(conn, GameStreamReliability::Unreliable) {
                    tracing::error!("Failed to initiate unreliable stream: {e:?}");
                }
                
                commands.remove_resource::<ConnectTimeout>();
            }

            GameNetworkEvent::StreamCreated(conn, stream) => {
                let Ok(player_id) = Uuid::parse_str(&session.player_id) else {
                    tracing::error!("Invalid player_id in session: '{}'", session.player_id);
                    continue;
                };

                if stream.is_reliable() {
                    tracing::info!("Reliable stream is ready! Registering client_id={player_id}");
                    
                    let connect_message = BrokerMessage::serialize_connect(player_id);
                    if let Err(e) = peer.send(&conn, &stream, connect_message.into()) {
                        tracing::error!("Failed to send broker Connect: {e:?}");
                    }

                    next_state.set(GameState::InGame);
                } 
                else {
                    let payload = serialize_position_payload(&PositionPayload {
                        entity_id: player_id,
                        position: Vec2 { x: 0.0, y: 0.0 },
                    });

                    let topic = Topic::Position.to_bytes();
                    let publish = BrokerMessage::serialize_publish(topic, &payload);
                    if let Err(e) = peer.send(&conn, &stream, publish.into()) {
                        tracing::error!("Failed to send initial Publish: {e:?}");
                    } else {
                        tracing::info!("Sent initial baseline Position Publish for player_id={player_id}");
                    }
                }
            }

            GameNetworkEvent::Disconnected(conn) => {
                tracing::warn!("Disconnected from server ({:?})", conn.connection_id);
                next_state.set(GameState::Login);
            }

            GameNetworkEvent::Error { inner, .. } => {
                tracing::error!("Network error: {inner}");
                next_state.set(GameState::Login);
            }

            _ => {}
        }
    }
}

fn tick_connect_timeout(
    mut commands: Commands,
    time: Res<Time>,
    timeout: Option<ResMut<ConnectTimeout>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(mut timeout) = timeout else {
        return;
    };

    if timeout.0.tick(time.delta()).just_finished() {
        tracing::warn!("Connection timed out — returning to login");
        commands.remove_resource::<ConnectTimeout>();
        next_state.set(GameState::Login);
    }
}

// ---------------------------------------------------------------------------
// InGame packet receive (position batches from the server)
// ---------------------------------------------------------------------------

/// Message emitted when a fresh `PositionBatch` arrives from the server.
#[derive(Message)]
pub struct PositionBatchReceived(pub PositionBatch);

fn receive_packets(
    peer_res: Option<ResMut<ActivePeer>>,
    mut batch_writer: MessageWriter<PositionBatchReceived>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else { return };

    while let Ok(Some(event)) = peer.poll() {
        if let GameNetworkEvent::Message { data, .. } = event {
            if let Some(message) = BrokerMessage::deserialize(&data) {
                match message {
                    BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
                        Topic::ShardSnapshot(_) => {
                            if let Some(snapshot) = deserialize_shard_snapshot_payload(&payload) {
                                emit_snapshot_batch(snapshot, &mut batch_writer);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                continue;
            }

            if let Ok(batch) = wincode::deserialize::<PositionBatch>(&data) {
                batch_writer.write(PositionBatchReceived(batch));
                continue;
            }

        }
    }
}

fn emit_snapshot_batch(
    snapshot: ShardSnapshotPayload,
    batch_writer: &mut MessageWriter<PositionBatchReceived>,
) {
    if let Ok(batch) = wincode::deserialize::<PositionBatch>(&snapshot.replication) {
        batch_writer.write(PositionBatchReceived(batch));
        return;
    }

    if let Ok(state) = wincode::deserialize::<ShardSnapshotState>(&snapshot.replication) {
        let batch = PositionBatch {
            tick: 0,
            snapshots: state
                .players
                .into_iter()
                .map(|player| PositionSnapshot {
                    entity_id: player.entity_id,
                    display_name: player.username,
                    x: 0.0,
                    y: 0.0,
                    vx: 0.0,
                    vy: 0.0,
                })
                .collect(),
        };
        batch_writer.write(PositionBatchReceived(batch));
    }
}