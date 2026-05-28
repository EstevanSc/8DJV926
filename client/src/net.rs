use std::sync::Mutex;

use bevy::prelude::*;
use common::broker_messages::BrokerMessage;
use common::topics::{serialize_position_payload, PositionPayload, Topic};
use common::Vec2;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

use common::packets::PositionBatch;

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
// Wire protocol — must match GameServer/src/messages.rs exactly.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
enum GameMessage {
    Join { username: String },
    Welcome { player_id: Uuid },
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Wraps the active `GamePeer` in a Mutex so it satisfies Bevy's Sync bound.
#[derive(Resource)]
pub struct ActivePeer(pub Mutex<GamePeer>);

/// Set to true once we have sent the `Join` message to the server.
#[derive(Resource, Default)]
struct JoinSent(bool);

/// Countdown started when we enter Connecting. If no `Connected` event arrives
/// before it expires the client returns to Login.
#[derive(Resource)]
struct ConnectTimeout(Timer);

/// The UUID assigned by the server in the `Welcome` message.
/// Also stored as a hash-derived `u32` for the position-interpolation system.
#[derive(Resource, Clone, Copy)]
pub struct MyEntityId(pub u32);

/// The active game-server connection handle. Inserted once the QUIC handshake
/// completes. Used by other systems (e.g. input) to send datagrams.
#[derive(Resource, Clone, Copy)]
pub struct ServerConn(pub GameConnection);

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
    commands.insert_resource(JoinSent(false));
    commands.insert_resource(ConnectTimeout(Timer::from_seconds(10.0, TimerMode::Once)));
}

fn poll_net_events(
    mut commands: Commands,
    session: Res<GameSession>,
    peer_res: Option<ResMut<ActivePeer>>,
    mut join_sent: ResMut<JoinSent>,
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
                tracing::info!("QUIC connected (id={:?}); sending Join", conn.connection_id);
                commands.insert_resource(ServerConn(conn));

                if let Ok(player_id) = Uuid::parse_str(&session.player_id) {
                    let payload = serialize_position_payload(&PositionPayload {
                        entity_id: player_id,
                        position: Vec2 { x: 0.0, y: 0.0 },
                    });

                    let topic = Topic::Position.to_bytes();
                    let publish = BrokerMessage::serialize_publish(topic, &payload);
                    let stream = GameStream::from(0);

                    if let Err(e) = peer.send(&conn, &stream, publish.into()) {
                        tracing::error!("Failed to send initial Publish: {e:?}");
                    } else {
                        tracing::info!("Sent initial Publish for player_id={player_id}");
                    }
                } else {
                    tracing::error!("Invalid player_id in session: '{}'", session.player_id);
                }

                if !join_sent.0 {
                    let msg = GameMessage::Join {
                        username: session.username.clone(),
                    };
                    match wincode::serialize(&msg) {
                        Ok(data) => {
                            let stream = GameStream::from(0);
                            if let Err(e) = peer.send(&conn, &stream, data.into()) {
                                tracing::error!("Failed to send Join: {e:?}");
                            } else {
                                tracing::info!("Sent Join for '{}'", session.username);
                                join_sent.0 = true;
                            }
                        }
                        Err(e) => tracing::error!("Failed to serialize Join: {e:?}"),
                    }
                }
            }

            GameNetworkEvent::Message { data, .. } => {
                match wincode::deserialize::<GameMessage>(&data) {
                    Ok(GameMessage::Welcome { player_id }) => {
                        tracing::info!("Received Welcome — player_id={player_id}");
                        // Derive a u32 from the UUID for the position-interpolation system.
                        let entity_id = player_id
                            .as_bytes()
                            .iter()
                            .fold(0u32, |acc, &b| acc.wrapping_add(b as u32));
                        commands.insert_resource(MyEntityId(entity_id));
                        next_state.set(GameState::InGame);
                    }
                    Ok(other) => {
                        tracing::warn!("Unexpected message variant: {other:?}");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to decode server message: {e:?}");
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
    mut timeout: ResMut<ConnectTimeout>,
    mut next_state: ResMut<NextState<GameState>>,
) {
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
            if let Ok(batch) = wincode::deserialize::<PositionBatch>(&data) {
                batch_writer.write(PositionBatchReceived(batch));
            }
        }
    }
}