use std::sync::Mutex;

use bevy::prelude::*;
use common::broker_messages::BrokerMessage;
use common::topics::{PositionPayload, StartingPositionPayload, Topic, deserialize_position_payload, serialize_starting_position_payload, QuadtreeBoundariesUpdatePayload, deserialize_quadtree_boundaries_update_payload};
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStreamReliability};

use super::{GameSession, GameState};

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PositionUpdateReceived>()
            .add_message::<QuadtreeBoundariesUpdateReceived>()
            .add_systems(OnEnter(GameState::Connecting), start_connect)
            .add_systems(
                Update,
                (poll_net_events, tick_connect_timeout).run_if(in_state(GameState::Connecting)),
            )
            .add_systems(Update, receive_packets.run_if(in_state(GameState::InGame)));
    }
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

/// The active broker connection handle. Inserted once the QUIC handshake
/// completes. Used by other systems (e.g. input) to send datagrams.
#[derive(Resource, Clone)]
pub struct BrokerConn(pub GameConnection);

/// Reliable control stream used for broker messages (connect/subscribe/publish).
#[derive(Resource, Clone)]
pub struct BrokerControlStream(pub game_sockets::GameStream);

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
    _session: Res<GameSession>,
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
                /* 
                let Ok(player_id) = Uuid::parse_str(&session.player_id) else {
                    tracing::error!("Invalid player_id in session: '{}'", session.player_id);
                    continue;
                };
                */
                let player_id = conn.connection_id;
                if stream.is_reliable() { 
                    tracing::info!("Reliable stream is ready! Registering client_id={player_id} with broker...");
                    commands.insert_resource(BrokerControlStream(stream.clone()));
                    
                    let connect_message = BrokerMessage::serialize_connect(player_id, common::broker_messages::SendingSystem::Client);
                    if let Err(e) = peer.send(&conn, &stream, connect_message.into()) {
                        tracing::error!("Failed to send broker Connect: {e:?}");
                    }

                    let subscribe_updates = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::EntityPositionUpdate(player_id).to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_updates.into()) {
                        tracing::error!("Failed to subscribe to EntityPositionUpdate: {e:?}");
                    } else {
                        tracing::info!("Subscribed to EntityPositionUpdate for player_id={player_id}");
                    }

                    let payload = serialize_starting_position_payload(&StartingPositionPayload {
                        connection_id: player_id,
                        position: [-50.0, -50.0],
                    });

                    let topic = Topic::PlayerStartingPosition.to_bytes();
                    let publish = BrokerMessage::serialize_publish(topic, &payload);
                    if let Err(e) = peer.send(&conn, &stream, publish.into()) {
                        tracing::error!("Failed to send initial Publish: {e:?}");
                    } else {
                        tracing::info!("Sent initial baseline StartingPosition Publish for player_id={player_id}");
                    }

                    let subscribe_quadtree = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::QuadtreeBoundariesUpdate.to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_quadtree.into()) {
                        tracing::error!("Failed to subscribe to QuadtreeBoundariesUpdate: {e:?}");
                    } else {
                        tracing::info!("Subscribed to QuadtreeBoundariesUpdate for player_id={player_id}");
                    }

                    next_state.set(GameState::InGame);
                } 
                else {

                }
            }

            GameNetworkEvent::Disconnected(conn) => {
                tracing::warn!("Disconnected from server ({:?})", conn.connection_id);
                commands.remove_resource::<BrokerControlStream>();
                next_state.set(GameState::Login);
            }

            GameNetworkEvent::Error { inner, .. } => {
                tracing::error!("Network error: {inner}");
                commands.remove_resource::<BrokerControlStream>();
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

/// Message emitted when an entity position update arrives from the server.
#[derive(Message)]
pub struct PositionUpdateReceived {
    pub connection_id: uuid::Uuid,
    pub payload: PositionPayload,
}
    
/// Message emitted when a quadtree boundaries update arrives from the server.
#[derive(Message)]
pub struct QuadtreeBoundariesUpdateReceived {
    pub payload: QuadtreeBoundariesUpdatePayload,
}

fn receive_packets(
    peer_res: Option<ResMut<ActivePeer>>,
    mut update_writer: MessageWriter<PositionUpdateReceived>,
    mut quadtree_update_writer: MessageWriter<QuadtreeBoundariesUpdateReceived>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else { return };

    while let Ok(Some(event)) = peer.poll() {
        if let GameNetworkEvent::Message { data, .. } = event {
            if let Some(message) = BrokerMessage::deserialize(&data) {
                match message {
                    BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
                        Topic::EntityPositionUpdate( entity_uuid) => {
                            tracing::debug!("Received position update for entity {:?}", entity_uuid);
                            if let Some(update) = deserialize_position_payload(&payload) {
                                tracing::trace!("Deserialized position update: {:?}", update);
                                update_writer.write(PositionUpdateReceived {
                                    connection_id: entity_uuid,
                                    payload: update,
                                });
                            }
                        }
                        Topic::QuadtreeBoundariesUpdate => {
                            tracing::info!("Received quadtree boundaries update from server");
                            if let Some(update) = deserialize_quadtree_boundaries_update_payload(&payload) {
                                tracing::trace!("Deserialized quadtree boundaries update: {:?}", update);
                                quadtree_update_writer.write(QuadtreeBoundariesUpdateReceived {
                                    payload: update,
                                });
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                continue;
            }
        }
    }
}