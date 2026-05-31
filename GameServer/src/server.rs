use crate::heartbeat::Heartbeat;
use crate::messages::GameMessage;
use crate::net::{ConnectedPlayers, SimCommandSender, entity_id_from_uuid};
use crate::authority::{AuthorityEnvelope, HandoffRequestState, GhostReplica};
use crate::authority::systems::AuthorityBus;

use common::broker_messages::BrokerMessage;
use common::packets::PlayerInput;
use common::packets::PositionBatch;
use common::topics::{
    deserialize_crossing_alert_payload, deserialize_handoff_request_payload,
    deserialize_handoff_complete_payload,
    deserialize_input_payload, deserialize_shard_snapshot_payload,
    serialize_handoff_request_payload, serialize_shard_created_payload,
    serialize_shard_snapshot_payload,
    HandoffRequestPayload, ShardCreatedPayload, ShardSnapshotPayload, Topic,
    deserialize_forced_position_update_payload,
};
use common::Vec2;
use bevy::prelude::*;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream, GameStreamReliability};
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;
use uuid::Uuid;

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ServerConfig::from_env())
            .init_resource::<PlayerRegistry>()
            .init_resource::<ShardUuidById>()
            .insert_resource(HeartbeatTimer(Timer::new(
                Duration::from_secs(5),
                TimerMode::Repeating,
            )))
            .add_systems(Startup, (bind_socket, connect_broker).chain())
            .add_systems(Update, (receive_packets, poll_broker_events, send_heartbeat, flush_authority_outbox).chain());
    }
}

#[derive(Resource, Default)]
pub struct ShardUuidById(pub HashMap<u32, Uuid>);

#[derive(Resource, Default)]
pub struct PlayerRegistry {
    pub registry: HashMap<Uuid, PlayerInfo>,
}

#[allow(dead_code)]
pub struct PlayerInfo {
    pub id: Uuid,
    pub entity_id: u32,
    pub username: String,
}

#[derive(Resource)]
pub struct ServerConfig {
    pub id: String,
    /// Bind address (always 0.0.0.0 inside the container).
    pub ip: String,
    /// Routable address advertised to clients via heartbeat.
    pub public_ip: String,
    pub port: u16,
    pub broker_host: String,
    pub broker_port: u16,
    pub zone: String,
    pub max_players: usize,
    pub orchestrator_address: SocketAddr,
}
impl ServerConfig {
    pub fn from_env() -> Self {
        let port = std::env::var("DS_PORT")
            .unwrap_or_else(|_| "7777".to_string())
            .parse::<u16>()
            .expect("Invalid DS_PORT");
        let broker_port = std::env::var("BROKER_PORT")
            .unwrap_or_else(|_| "7776".to_string())
            .parse::<u16>()
            .expect("Invalid BROKER_PORT");

        let orchestrator_host =
            std::env::var("ORCH_HOST").unwrap_or_else(|_| "127.0.0.1:7000".to_string());
        let orchestrator_address: SocketAddr = orchestrator_host
            .to_socket_addrs()
            .expect("Invalid orchestrator address")
            .next()
            .expect("No addresses resolved for ORCH_HOST");

        Self {
            // When spawned by the orchestrator DS_ID is injected so the heartbeat
            // key matches the Redis entry created during container spawn.
            id: std::env::var("DS_ID").unwrap_or_else(|_| Uuid::new_v4().to_string()),
            ip: "0.0.0.0".to_string(),
            // DS_PUBLIC_IP is the address clients use to reach this server.
            // Set to "localhost" for local Docker dev (port-mapped to host).
            public_ip: std::env::var("DS_PUBLIC_IP").unwrap_or_else(|_| "localhost".to_string()),
            port,
            broker_host: std::env::var("BROKER_HOST").unwrap_or_else(|_| "localhost".to_string()),
            broker_port,
            zone: std::env::var("DS_ZONE").unwrap_or_else(|_| "zone_A".to_string()),
            max_players: std::env::var("MAX_PLAYERS")
                .unwrap_or_else(|_| "2".to_string()) // low number to test FULL states easily
                .parse::<usize>()
                .unwrap(),
            orchestrator_address,
        }
    }
}

#[derive(Resource)]
pub struct NetworkPeer {
    pub peer: GamePeer,
}

#[derive(Resource)]
pub struct BrokerPeer {
    pub peer: GamePeer,
    pub connection: Option<GameConnection>,
    pub control_stream: Option<GameStream>,
    pub snapshot_stream: Option<GameStream>,
    pub shard_uuid: Option<Uuid>,
}

#[derive(Resource)]
pub struct HeartbeatTimer(pub Timer);

fn bind_socket(mut commands: Commands, server_config: Res<ServerConfig>) {
    let peer = GamePeer::new(QuicBackend::new());

    let ip = &server_config.ip;
    let port = server_config.port;

    match peer.listen(ip, port) {
        Ok(_) => {
            println!("Listening on {}", ip);
            commands.insert_resource(NetworkPeer { peer });
        }
        Err(e) => {
            eprintln!("Failed to listen on {}: {}", ip, e);
        }
    }
}

fn connect_broker(mut commands: Commands, server_config: Res<ServerConfig>) {
    let peer = GamePeer::new(QuicBackend::new());

    match peer.connect(&server_config.broker_host, server_config.broker_port) {
        Ok(_) => {
            println!(
                "Connecting to broker at {}:{}",
                server_config.broker_host, server_config.broker_port
            );
            commands.insert_resource(BrokerPeer {
                peer,
                connection: None,
                control_stream: None,
                snapshot_stream: None,
                shard_uuid: None,
            });
        }
        Err(e) => {
            eprintln!(
                "Failed to initiate broker connection to {}:{}: {}",
                server_config.broker_host, server_config.broker_port, e
            );
        }
    }
}

fn poll_broker_events(
    mut broker: ResMut<BrokerPeer>,
    mut server: ResMut<NetworkPeer>,
    mut player_registry: ResMut<PlayerRegistry>,
    sim_tx: Res<SimCommandSender>,
    mut authority_bus: ResMut<AuthorityBus>,
    mut shard_map: ResMut<ShardUuidById>,
) {
    while let Ok(Some(event)) = broker.peer.poll() {
        match event {
            GameNetworkEvent::Connected(connection) => {
                println!(
                    "Broker connection established! Connection id: {:?}",
                    connection.connection_id
                );
                broker.connection = Some(connection);
                if let Err(e) = broker.peer.create_stream(connection, GameStreamReliability::Reliable) {
                    eprintln!("Failed to create broker control stream: {:?}", e);
                }
                if let Err(e) = broker.peer.create_stream(connection, GameStreamReliability::Unreliable) {
                    eprintln!("Failed to create broker snapshot stream: {:?}", e);
                }
            }
            GameNetworkEvent::Message {
                data,
                connection,
                stream,
            } => {
                handle_message(
                    &data,
                    connection,
                    stream,
                    &mut server,
                    &mut player_registry,
                    &sim_tx,
                    Some(&mut broker),
                    &mut authority_bus,
                    &mut shard_map,
                );
            }
            GameNetworkEvent::StreamCreated(connection, stream) => {
                if broker.connection == Some(connection) {
                    if stream.is_reliable() && broker.control_stream.is_none() {
                        broker.control_stream = Some(stream);
                    } else if !stream.is_reliable() && broker.snapshot_stream.is_none() {
                        broker.snapshot_stream = Some(stream);
                    }

                    try_announce_shard_creation(&mut broker);
                }
            }
            GameNetworkEvent::Disconnected(connection) => {
                eprintln!(
                    "Broker disconnected! Connection id: {:?}",
                    connection.connection_id
                );
                broker.connection = None;
                broker.control_stream = None;
                broker.snapshot_stream = None;
            }
            GameNetworkEvent::Error { inner, .. } => {
                eprintln!("Broker network error: {:?}", inner);
            }
            _ => {}
        }
    }
}

fn receive_packets(
    mut server: ResMut<NetworkPeer>,
    mut player_registry: ResMut<PlayerRegistry>,
    sim_tx: Res<SimCommandSender>,
    conn_players: ResMut<ConnectedPlayers>,
    mut authority_bus: ResMut<AuthorityBus>,
    mut shard_map: ResMut<ShardUuidById>,
) {
    while let Ok(Some(event)) = server.peer.poll() {
        match event {
            GameNetworkEvent::Connected(conn) => {
                println!("Connected! Client id: {:?}", conn.connection_id);
                if let Ok(mut map) = conn_players.0.lock() {
                    map.insert(conn.connection_id, conn);
                }
            }
            GameNetworkEvent::Message {
                data,
                connection,
                stream,
            } => {
                handle_message(
                    &data,
                    connection,
                    stream,
                    &mut server,
                    &mut player_registry,
                    &sim_tx,
                    None,
                    &mut authority_bus,
                    &mut shard_map,
                );
            }
            GameNetworkEvent::Disconnected(conn) => {
                if let Ok(mut map) = conn_players.0.lock() {
                    map.remove(&conn.connection_id);
                }
                if let Some(info) = player_registry.registry.remove(&conn.connection_id) {
                    let _ = sim_tx.0.send(crate::net::SimCommand::Left {
                        entity_id: info.entity_id,
                    });
                }
                println!("Disconnected! Client id: {:?}", conn.connection_id);
            }
            _ => {}
        }
    }
}

fn try_announce_shard_creation(broker: &mut ResMut<BrokerPeer>) {
    if broker.shard_uuid.is_some() {
        return;
    }

    let (Some(connection), Some(control_stream)) = (
        broker.connection,
        broker.control_stream.clone(),
    ) else {
        return;
    };

    let shard_uuid = Uuid::new_v4();
    broker.shard_uuid = Some(shard_uuid);

    let connect_message = BrokerMessage::serialize_connect(shard_uuid);
    if let Err(e) = broker.peer.send(&connection, &control_stream, connect_message.into()) {
        eprintln!("Failed to send broker Connect message: {:?}", e);
        return;
    }

    let shard_created_payload = ShardCreatedPayload {
        shard_id: shard_uuid,
        center: Vec2 { x: 0.0, y: 0.0 },
    };
    let publish_message = BrokerMessage::serialize_publish(
        Topic::ShardCreated.to_bytes(),
        &serialize_shard_created_payload(&shard_created_payload),
    );

    if let Err(e) = broker.peer.send(&connection, &control_stream, publish_message.into()) {
        eprintln!("Failed to send broker ShardCreated publish: {:?}", e);
        return;
    }

    // Subscribe to handoff-related topics for this shard so we can receive crossing alerts and handoff requests from the quadtree, and send handoff accept/reject messages back.
    for topic in [
        Topic::CrossingAlert(shard_uuid),
        Topic::HandoffRequest(shard_uuid),
        Topic::HandoffAccept(shard_uuid),
        Topic::HandoffReject(shard_uuid),
        //Topic::GhostUpdate(shard_uuid),
        Topic::HandoffComplete(shard_uuid),
    ] {
        let sub = BrokerMessage::serialize_subscribe(shard_uuid, topic.to_bytes());
        let _ = broker.peer.send(&connection, &control_stream, sub.into());
    }

    println!("Announced shard creation to broker with shard_uuid={}", shard_uuid);
}

fn handle_message(
    data: &[u8],
    connection: game_sockets::GameConnection,
    stream: GameStream,
    server: &mut ResMut<NetworkPeer>,
    player_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
    broker: Option<&mut ResMut<BrokerPeer>>,
    authority_bus: &mut ResMut<AuthorityBus>,
    _shard_map: &mut ResMut<ShardUuidById>,
) {
    if let Some(message) = BrokerMessage::deserialize(data) {
        handle_broker_message(
            message,
            connection,
            stream,
            player_registry,
            sim_tx,
            broker,
            authority_bus,
        );
        return;
    }

    if let Ok(msg) = wincode::deserialize::<GameMessage>(data) {
        match msg {
            GameMessage::Join { username } => {
                println!("Joined {}", username);
                let id = connection.connection_id;
                let entity_id = entity_id_from_uuid(id);
                player_registry.registry.insert(
                    id,
                    PlayerInfo { id, entity_id, username: username.clone() },
                );
                let _ = sim_tx.0.send(crate::net::SimCommand::Joined {
                    entity_id,
                    display_name: username,
                    position: Vec2 { x: 0.0, y: 0.0 }
                });

                // Send Welcome message to the player
                let response = GameMessage::Welcome { player_id: id };
                if let Ok(serialized) = wincode::serialize(&response) {
                    server
                        .peer
                        .send(&connection, &stream, serialized.into())
                        .unwrap();
                } else {
                    eprintln!("Failed to serialize game message");
                }
            }
            _ => {
                println!("Unexpected message {:?}", msg);
            }
        }
        return;
    }

    if let Ok(input) = wincode::deserialize::<PlayerInput>(data) {
        // Unreliable player-input datagram
        handle_player_input(connection.connection_id, input.dx, input.dy, player_registry, sim_tx);
        return;
    }

    eprintln!("Unknown message from {:?}", connection.connection_id);
}

fn handle_broker_message(
    message: BrokerMessage,
    connection: game_sockets::GameConnection,
    _stream: GameStream,
    player_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
    broker: Option<&mut ResMut<BrokerPeer>>,
    authority_bus: &mut ResMut<AuthorityBus>,
) {
    match message {
        BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
            Topic::CrossingAlert(source_shard_uuid) => {
                if let Some(crossing_alert) = deserialize_crossing_alert_payload(&payload) {
                    let entity_id = entity_id_from_uuid(crossing_alert.entity_uuid);
                    let _ = sim_tx.0.send(crate::net::SimCommand::CrossingAlert {
                        entity_id,
                        target_shard_uuid: crossing_alert.target_shard_uuid,
                    });
                    tracing::trace!(
                        source_shard_uuid = ?source_shard_uuid,
                        target_shard_uuid = ?crossing_alert.target_shard_uuid,
                        entity_id,
                        "Received crossing alert"
                    );
                } else {
                    eprintln!("Failed to decode crossing alert payload from {:?}", connection.connection_id);
                }
            }
            Topic::Input(player_id) => {
                if let Some(input) = deserialize_input_payload(&payload) {
                    handle_player_input(
                        player_id,
                        input.dxdy.x as f32,
                        input.dxdy.y as f32,
                        player_registry,
                        sim_tx,
                    );
                } else {
                    eprintln!("Failed to decode broker input payload from {:?}", connection.connection_id);
                }
            }
            Topic::Disconnect(player_id) => {
                trace!("Received disconnect broadcast for player_id={}", player_id);
                if let Some(info) = player_registry.registry.remove(&player_id) {
                    let _ = sim_tx.0.send(crate::net::SimCommand::Left {
                        entity_id: info.entity_id,
                    });
                }
            }
            Topic::ForcedPositionUpdate(player_id) => {
                if let Some(position_update) = deserialize_forced_position_update_payload(&payload) {
                    handle_player_position_update(
                        player_id,
                        position_update.position.x as f32,
                        position_update.position.y as f32,
                        player_registry,
                        sim_tx,
                    );
                } else {
                    eprintln!("Failed to decode broker forced position update payload from {:?}", connection.connection_id);
                }
            }
            Topic::HandoffRequest(target_shard_uuid) => {
                if let Some(request) = deserialize_handoff_request_payload(&payload) {
                    let player_count = player_registry.registry.len();
                    // Temporary capacity gate; replace with shard load/scaling policy later.
                    let allow_handoff = player_count < 100;

                    if let Some(broker) = broker {
                        let response_topic = if allow_handoff {
                            Topic::HandoffAccept(request.source_shard_uuid)
                        } else {
                            Topic::HandoffReject(request.source_shard_uuid)
                        };

                        let response_message = BrokerMessage::serialize_publish(
                            response_topic.to_bytes(),
                            &serialize_handoff_request_payload(&request),
                        );

                        let _ = broker.peer.send(&connection, &_stream, response_message.into());
                    }

                    tracing::info!(
                        target_shard_uuid = ?target_shard_uuid,
                        source_shard_uuid = ?request.source_shard_uuid,
                        entity_id = request.entity_id,
                        player_count,
                        allow_handoff,
                        "Processed handoff request"
                    );
                } else {
                    eprintln!("Failed to decode handoff request payload from {:?}", connection.connection_id);
                }
            }
            Topic::HandoffAccept(_) => {
                if let Some(request) = deserialize_handoff_request_payload(&payload) {
                    authority_bus
                        .inbound
                        .push_back(AuthorityEnvelope::HandoffAccept(crate::authority::HandoffAccept {
                            entity_id: request.entity_id,
                        }));
                } else {
                    eprintln!("Failed to decode handoff accept payload from {:?}", connection.connection_id);
                }
            }
            Topic::HandoffReject(_) => {
                if let Some(request) = deserialize_handoff_request_payload(&payload) {
                    authority_bus
                        .inbound
                        .push_back(AuthorityEnvelope::HandoffReject(crate::authority::HandoffReject {
                            entity_id: request.entity_id,
                        }));
                } else {
                    eprintln!("Failed to decode handoff reject payload from {:?}", connection.connection_id);
                }
            }
            Topic::HandoffComplete(_) => {
                if let Some(complete) = deserialize_handoff_complete_payload(&payload) {
                    tracing::info!(
                        entity_id = complete.entity_id,
                        source_shard_id = ?complete.source_shard_id,
                        target_shard_id = ?complete.target_shard_id,
                        result = ?complete.result,
                        "Received handoff complete"
                    );

                    if complete.result.is_transfer() {
                        authority_bus
                            .inbound
                            .push_back(AuthorityEnvelope::HandoffComplete(crate::authority::HandoffComplete {
                                entity_id: complete.entity_id,
                            }));
                    }
                } else {
                    eprintln!("Failed to decode handoff complete payload from wire");
                }
            }
            Topic::GhostUpdate(_) => {
                if let Some(position_update) = deserialize_forced_position_update_payload(&payload) {
                    let entity_id = entity_id_from_uuid(position_update.entity_id);
                    authority_bus.inbound.push_back(AuthorityEnvelope::GhostUpdate(
                        crate::authority::GhostUpdate {
                            entity_id,
                            pos: bevy::prelude::Vec2::new(
                                position_update.position.x as f32,
                                position_update.position.y as f32,
                            ),
                        },
                    ));
                    println!("Received ghost update for entity_id={} from shard, pos=({:.1}, {:.1})", entity_id, position_update.position.x, position_update.position.y);
                } else {
                    eprintln!("Failed to decode ghost update payload from wire");
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn flush_authority_outbox(
    mut bus: ResMut<AuthorityBus>,
    broker: ResMut<BrokerPeer>,
    query: Query<(&crate::simulation::Player, &HandoffRequestState)>,
    ghost_query: Query<(&crate::simulation::Player, &GhostReplica)>,
) {
    let (Some(conn), Some(stream)) = (broker.connection, broker.control_stream.clone()) else {
        return;
    };

    let self_uuid = broker.shard_uuid.unwrap_or_default();

    while let Some(message) = bus.outbound.pop_front() {
        let mut raw_bytes = message.encode().to_vec();
        
        let target_uuid = match &message {
            AuthorityEnvelope::HandoffRequest(req) => {
                if let Some((_, request_state)) = query.iter().find(|(p, _)| p.entity_id == req.entity_id) {
                    let payload = HandoffRequestPayload {
                        source_shard_uuid: self_uuid,
                        entity_id: req.entity_id,
                        target_shard_uuid: request_state.target_shard_uuid,
                        position: common::Vec2 { x: req.pos.x as f64, y: req.pos.y as f64 },
                        velocity: common::Vec2 { x: req.vel.x as f64, y: req.vel.y as f64 },
                        state: req.state.to_vec(),
                    };

                    raw_bytes = serialize_handoff_request_payload(&payload);
                    request_state.target_shard_uuid
                } else {
                    Uuid::nil()
                }
            },
            AuthorityEnvelope::GhostUpdate(update) => {
                query.iter()
                    .find(|(p, _)| p.entity_id == update.entity_id)
                    .and_then(|(_, s)| Some(s.target_shard_uuid))
                    .unwrap_or(Uuid::nil())
            },
            AuthorityEnvelope::HandoffComplete(complete) => {
                ghost_query.iter()
                    .find(|(p, _)| p.entity_id == complete.entity_id)
                        .and_then(|(_, _g)| Some(Uuid::nil()))
                    .unwrap_or(Uuid::nil())
            },
            _ => Uuid::nil(),
        };

        if target_uuid.is_nil() {
            continue;
        }

        let topic = match message {
            AuthorityEnvelope::HandoffRequest(_) => Topic::HandoffRequest(target_uuid),
            AuthorityEnvelope::HandoffAccept(_) => Topic::HandoffAccept(target_uuid),
            AuthorityEnvelope::HandoffReject(_) => Topic::HandoffReject(target_uuid),
            AuthorityEnvelope::GhostUpdate(_) => Topic::GhostUpdate(target_uuid),
            AuthorityEnvelope::HandoffComplete(_) => Topic::HandoffComplete(target_uuid),
        };

        let pub_msg = BrokerMessage::serialize_publish(topic.to_bytes(), &raw_bytes);
        let _ = broker.peer.send(&conn, &stream, pub_msg.into());
    }
}

pub(crate) fn publish_shard_snapshot(broker: &mut ResMut<BrokerPeer>, batch: &PositionBatch) {
    let Some(connection) = broker.connection else {
        eprintln!("Cannot publish shard snapshot: no broker connection");
        return;
    };

    let Some(stream) = broker.snapshot_stream.clone() else {
        eprintln!("Cannot publish shard snapshot: no broker snapshot stream");
        return;
    };

    let Some(shard_uuid) = broker.shard_uuid else {
        eprintln!("Cannot publish shard snapshot: shard UUID not set");
        return;
    };

    let Ok(snapshot_bytes) = wincode::serialize(batch) else {
        eprintln!("Failed to serialize shard snapshot state");
        return;
    };

    let payload = ShardSnapshotPayload {
        shard_id: shard_uuid,
        replication: snapshot_bytes,
    };

    let payload_bytes = serialize_shard_snapshot_payload(&payload);
    if deserialize_shard_snapshot_payload(&payload_bytes).is_none() {
        eprintln!("Failed to validate shard snapshot payload round-trip");
        return;
    }

    let publish_message = BrokerMessage::serialize_publish(
        Topic::ShardSnapshot(shard_uuid).to_bytes(),
        &payload_bytes,
    );

    if let Err(e) = broker.peer.send(&connection, &stream, publish_message.into()) {
        eprintln!("Failed to send broker ShardSnapshot publish: {:?}", e);
    }
}


fn handle_player_input(
    player_id: Uuid,
    dx: f32,
    dy: f32,
    player_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
    // Auto-Join if unknown
    if !player_registry.registry.contains_key(&player_id) {
        let entity_id = crate::net::entity_id_from_uuid(player_id);
        let username = format!("Player_{}", entity_id);

        player_registry.registry.insert(
            player_id,
            PlayerInfo { id: player_id, entity_id, username: username.clone() },
        );

    }

    if let Some(info) = player_registry.registry.get(&player_id) {
        let _ = sim_tx.0.send(crate::net::SimCommand::Input {
            entity_id: info.entity_id,
            dx,
            dy,
        });
    }
}

fn handle_player_position_update(
    player_id: Uuid,
    x: f32,
    y: f32,
    player_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
        // Auto-Join if unknown
    if !player_registry.registry.contains_key(&player_id) {
        println!("Premier input de {}, auto-join sur ce shard !", player_id);
        
        let entity_id = crate::net::entity_id_from_uuid(player_id);
        let username = format!("Player_{}", entity_id);

        player_registry.registry.insert(
            player_id,
            PlayerInfo { id: player_id, entity_id, username: username.clone() },
        );

        let _ = sim_tx.0.send(crate::net::SimCommand::Joined {
            entity_id,
            display_name: username,
            position: Vec2 { x: x as f64, y: y as f64 },
        });
    }
}

fn send_heartbeat(
    time: Res<Time>,
    mut timer: ResMut<HeartbeatTimer>,
    players: Res<PlayerRegistry>,
    config: Res<ServerConfig>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        let player_count = players.registry.len();

        let heartbeat_data = Heartbeat {
            id: config.id.clone(),
            ip: config.public_ip.clone(),
            port: config.port,
            zone: config.zone.clone(),
            player_count,
            max_players: config.max_players,
        };

        // Send heartbeat JSON packet to the orchestrator
        if let Ok(json_payload) = serde_json::to_string(&heartbeat_data)
            && let Ok(udp_socket) = UdpSocket::bind("0.0.0.0:0")
        {
            let bytes = json_payload.as_bytes();
            if let Err(e) = udp_socket.send_to(bytes, config.orchestrator_address) {
                eprintln!("Failed to send heartbeat packet: {:?}", e);
            } else {
                println!(
                    "Heartbeat sent: {}/{} players. Status: {}",
                    player_count,
                    config.max_players,
                    if player_count >= config.max_players {
                        "FULL"
                    } else {
                        "AVAILABLE"
                    }
                );
            }
        }
    }}

