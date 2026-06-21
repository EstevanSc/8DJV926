use std::sync::Mutex;

use bevy::prelude::*;
use common::broker_messages::BrokerMessage;
use common::topics::{
    AuthorityDebugPacketPayload, PositionPayload, QuadtreeBoundariesUpdatePayload,
    StartingPositionPayload, Topic, deserialize_authority_debug_packet_payload,
    deserialize_db_name_response_payload, deserialize_path_response_payload,
    deserialize_position_payload, deserialize_quadtree_boundaries_update_payload,
    deserialize_use_ability_payload, serialize_starting_position_payload,
    deserialize_attribute_updated_payload,
};
use common::attribute_type::AttributeType;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStreamReliability};

use super::{GameSession, GameState};

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PositionUpdateReceived>()
            .add_message::<QuadtreeBoundariesUpdateReceived>()
            .add_message::<AuthorityDebugPacketReceived>()
            .add_message::<DisconnectReceived>()
            .add_message::<PathResponseReceived>()
            .add_message::<AbilityCastReceived>()
            .add_message::<DbNameResponseReceived>()
            .add_message::<LocalPlayerKilled>()
            .add_message::<AttributeUpdatedReceived>()
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
    session: Res<GameSession>,
    peer_res: Option<ResMut<ActivePeer>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else {
        return;
    };

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
                tracing::info!(
                    "QUIC connected (id={:?}); opening streams...",
                    conn.connection_id
                );
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
                    tracing::info!(
                        "Reliable stream is ready! Registering client_id={player_id} with broker..."
                    );
                    commands.insert_resource(BrokerControlStream(stream.clone()));

                    let connect_message = BrokerMessage::serialize_connect(
                        player_id,
                        common::broker_messages::SendingSystem::Client,
                    );
                    if let Err(e) = peer.send(&conn, &stream, connect_message.into()) {
                        tracing::error!("Failed to send broker Connect: {e:?}");
                    }

                    // Send database username correlation registration
                    let register_payload = common::topics::serialize_db_register_username_payload(
                        &common::topics::DbRegisterUsernamePayload {
                            player_id,
                            username: session.username.clone(),
                        },
                    );
                    let register_publish = BrokerMessage::serialize_publish(
                        Topic::DbRegisterUsername.to_bytes(),
                        &register_payload,
                    );
                    if let Err(e) = peer.send(&conn, &stream, register_publish.into()) {
                        tracing::error!("Failed to send DbRegisterUsername: {e:?}");
                    } else {
                        tracing::info!(
                            "Sent DbRegisterUsername mapping for player_id={player_id}, username={}",
                            session.username
                        );
                    }

                    let subscribe_updates = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::EntityPositionUpdate(player_id).to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_updates.into()) {
                        tracing::error!("Failed to subscribe to EntityPositionUpdate: {e:?}");
                    } else {
                        tracing::info!(
                            "Subscribed to EntityPositionUpdate for player_id={player_id}"
                        );
                    }

                    let payload = serialize_starting_position_payload(&StartingPositionPayload {
                        connection_id: player_id,
                        position: [
                            session.player_spawn_position[0] as f64,
                            session.player_spawn_position[1] as f64,
                        ],
                    });

                    let topic = Topic::PlayerStartingPosition.to_bytes();
                    let publish = BrokerMessage::serialize_publish(topic, &payload);
                    if let Err(e) = peer.send(&conn, &stream, publish.into()) {
                        tracing::error!("Failed to send initial Publish: {e:?}");
                    } else {
                        tracing::info!(
                            "Sent initial baseline StartingPosition Publish for player_id={player_id}"
                        );
                    }

                    let subscribe_quadtree = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::QuadtreeBoundariesUpdate.to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_quadtree.into()) {
                        tracing::error!("Failed to subscribe to QuadtreeBoundariesUpdate: {e:?}");
                    } else {
                        tracing::info!(
                            "Subscribed to QuadtreeBoundariesUpdate for player_id={player_id}"
                        );
                    }

                    let subscribe_debug = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::AuthorityDebugPacket(player_id).to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_debug.into()) {
                        tracing::error!("Failed to subscribe to AuthorityDebugPacket: {e:?}");
                    } else {
                        tracing::info!(
                            "Subscribed to AuthorityDebugPacket for player_id={player_id}"
                        );
                    }

                    let subscribe_path_response = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::PathResponse(player_id).to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_path_response.into()) {
                        tracing::error!("Failed to subscribe to PathResponse: {e:?}");
                    } else {
                        tracing::info!("Subscribed to PathResponse for player_id={player_id}");
                    }

                    let subscribe_entity_killed = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::EntityKilled(player_id).to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_entity_killed.into()) {
                        tracing::error!("Failed to subscribe to EntityKilled: {e:?}");
                    } else {
                        tracing::info!("Subscribed to EntityKilled for player_id={player_id}");
                    }

                    let subscribe_attribute_updated = BrokerMessage::serialize_subscribe(
                        player_id,
                        Topic::AttributeUpdated(player_id).to_bytes(),
                    );
                    if let Err(e) = peer.send(&conn, &stream, subscribe_attribute_updated.into()) {
                        tracing::error!("Failed to subscribe to AttributeUpdated: {e:?}");
                    } else {
                        tracing::info!("Subscribed to AttributeUpdated for player_id={player_id}");
                    }

                    next_state.set(GameState::InGame);
                } else {
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

#[derive(Message)]
pub struct AuthorityDebugPacketReceived {
    pub payload: AuthorityDebugPacketPayload,
}

#[derive(Message)]
pub struct PathResponseReceived {
    pub path: Vec<Vec2>,
}

#[derive(Message)]
pub struct DisconnectReceived {
    pub entity_id: uuid::Uuid,
}

#[derive(Message)]
pub struct AbilityCastReceived {
    pub caster_id: uuid::Uuid,
    pub ability_type: common::ability_type::AbilityType,
    pub direction: Option<Vec2>,
}

#[derive(Message)]
pub struct DbNameResponseReceived {
    pub player_id: uuid::Uuid,
    pub username: String,
}

#[derive(Message)]
pub struct LocalPlayerKilled;

#[derive(Message)]
pub struct AttributeUpdatedReceived {
    pub entity_id: uuid::Uuid,
    pub attribute: AttributeType,
    pub new_value: i32,
}

fn receive_packets(
    peer_res: Option<ResMut<ActivePeer>>,
    broker_conn: Option<Res<BrokerConn>>,
    //broker_stream: Option<Res<BrokerControlStream>>,
    mut update_writer: MessageWriter<PositionUpdateReceived>,
    mut quadtree_update_writer: MessageWriter<QuadtreeBoundariesUpdateReceived>,
    mut authority_debug_writer: MessageWriter<AuthorityDebugPacketReceived>,
    mut disconnect_writer: MessageWriter<DisconnectReceived>,
    mut path_response_writer: MessageWriter<PathResponseReceived>,
    mut ability_cast_writer: MessageWriter<AbilityCastReceived>,
    mut name_response_writer: MessageWriter<DbNameResponseReceived>,
    mut local_player_killed_writer: MessageWriter<LocalPlayerKilled>,
    mut attribute_updated_writer: MessageWriter<AttributeUpdatedReceived>,
    _session: Res<GameSession>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else {
        return;
    };
    //let mut killed = false;
    let Some(conn) = broker_conn.as_ref() else {
        return;
    };
    let self_id = conn.0.connection_id;
    while let Ok(Some(event)) = peer.poll() {
        if let GameNetworkEvent::Message { data, .. } = event {
            if let Some(message) = BrokerMessage::deserialize(&data) {
                match message {
                    BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
                        Topic::EntityPositionUpdate(entity_uuid) => {
                            //tracing::debug!("Received position update for entity {:?}", entity_uuid);
                            if let Some(update) = deserialize_position_payload(&payload) {
                                //tracing::trace!("Deserialized position update: {:?}", update);
                                update_writer.write(PositionUpdateReceived {
                                    connection_id: entity_uuid,
                                    payload: update,
                                });
                            }
                        }
                        Topic::QuadtreeBoundariesUpdate => {
                            //tracing::info!("Received quadtree boundaries update from server");
                            if let Some(update) =
                                deserialize_quadtree_boundaries_update_payload(&payload)
                            {
                                //tracing::trace!("Deserialized quadtree boundaries update: {:?}", update);
                                quadtree_update_writer
                                    .write(QuadtreeBoundariesUpdateReceived { payload: update });
                            }
                        }
                        Topic::AuthorityDebugPacket(_entity_uuid) => {
                            //tracing::info!("Received authority debug packet from server for entity {:?}", entity_uuid);
                            if let Some(update) =
                                deserialize_authority_debug_packet_payload(&payload)
                            {
                                //tracing::trace!("Deserialized authority debug packet: {:?}", update);
                                authority_debug_writer
                                    .write(AuthorityDebugPacketReceived { payload: update });
                            }
                        }
                        Topic::Disconnect(uuid) => {
                            tracing::info!("Received disconnect message for entity {:?}", uuid);
                            disconnect_writer.write(DisconnectReceived { entity_id: uuid });
                        }
                        Topic::EntityKilled(uuid) => {
                            if uuid == self_id {
                                tracing::info!(
                                    "Received EntityKilled for our own entity {:?} — we were killed!",
                                    uuid
                                );
                                local_player_killed_writer.write(LocalPlayerKilled);
                            }
                        }
                        Topic::AttributeUpdated(uuid) => {
                            if let Some(payload) = deserialize_attribute_updated_payload(&payload) {
                                tracing::info!(
                                    "Received AttributeUpdated for entity {:?}: {:?} = {}",
                                    uuid,
                                    payload.attribute,
                                    payload.new_value
                                );
                                attribute_updated_writer.write(AttributeUpdatedReceived {
                                    entity_id: uuid,
                                    attribute: payload.attribute,
                                    new_value: payload.new_value,
                                });
                            }
                        }
                        Topic::PathResponse(_entity_uuid) => {
                            tracing::info!(
                                "Received path response from server for entity {:?}",
                                _entity_uuid
                            );
                            if let Some(response) = deserialize_path_response_payload(&payload) {
                                tracing::trace!("Deserialized path response: {:?}", response);
                                let mut path = Vec::new();
                                for point in response.path {
                                    path.push(Vec2::new(point[0], point[1]));
                                    print!("Added point {:?} to path", path.last());
                                }
                                path_response_writer.write(PathResponseReceived { path: path });
                            }
                        }
                        Topic::CastAbility(caster_uuid) => {
                            if let Some(payload) = deserialize_use_ability_payload(&payload) {
                                let dir = payload.direction.map(|d| Vec2::new(d[0], d[1]));
                                ability_cast_writer.write(AbilityCastReceived {
                                    caster_id: caster_uuid,
                                    ability_type: payload.ability,
                                    direction: dir,
                                });
                            } else {
                                tracing::error!("Couldn't deserialize CastABility payload.");
                            }
                        }
                        Topic::DbNameResponse(uuid) => {
                            if let Some(payload) = deserialize_db_name_response_payload(&payload) {
                                name_response_writer.write(DbNameResponseReceived {
                                    player_id: uuid,
                                    username: payload.username,
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

    /*if killed {
        if let (Some(conn), Some(stream)) = (broker_conn, broker_stream) {
            let respawn_entity_id = conn.0.connection_id;
            let payload = serialize_starting_position_payload(&StartingPositionPayload {
                connection_id: respawn_entity_id,
                position: [0.0, 0.0],
            });
            let topic = Topic::PlayerStartingPosition.to_bytes();
            let publish = BrokerMessage::serialize_publish(topic, &payload);

            // Utilise maintenant 'stream.0'
            if let Ok(peer) = peer_res.0.lock() {
                let _ = peer.send(&conn.0, &stream.0, publish.into());
            }
        }
    }*/
}
