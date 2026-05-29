use crate::heartbeat::Heartbeat;
use crate::messages::GameMessage;
use crate::net::{ConnectedPlayers, SimCommandSender, entity_id_from_uuid};
use common::broker_messages::BrokerMessage;
use common::packets::PlayerInput;
use common::packets::PositionBatch;
use common::topics::{
    deserialize_input_payload, deserialize_shard_snapshot_payload, serialize_shard_created_payload,
    serialize_shard_snapshot_payload, ShardCreatedPayload, ShardSnapshotPayload, Topic,
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
            .insert_resource(HeartbeatTimer(Timer::new(
                Duration::from_secs(5),
                TimerMode::Repeating,
            )))
            .add_systems(Startup, (bind_socket, connect_broker).chain())
            .add_systems(Update, (receive_packets, poll_broker_events, send_heartbeat).chain());
    }
}

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

fn poll_broker_events(mut broker: ResMut<BrokerPeer>) {
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

    println!("Announced shard creation to broker with shard_uuid={}", shard_uuid);
}

fn handle_message(
    data: &[u8],
    connection: game_sockets::GameConnection,
    stream: GameStream,
    server: &mut ResMut<NetworkPeer>,
    player_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
    if let Some(message) = BrokerMessage::deserialize(data) {
        handle_broker_message(message, connection, stream, player_registry, sim_tx);
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
) {
    match message {
        BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
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
            other => {
                println!("Ignoring broker broadcast for unexpected topic {:?}", other);
            }
        },
        _ => {
            println!("Ignoring broker message {:?} from {:?}", message, connection.connection_id);
        }
    }
}

pub(crate) fn publish_shard_snapshot(broker: &mut ResMut<BrokerPeer>, batch: &PositionBatch) {
    let Some(connection) = broker.connection else {
        return;
    };

    let Some(stream) = broker.snapshot_stream.clone() else {
        return;
    };

    let Some(shard_uuid) = broker.shard_uuid else {
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
    player_registry: &ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
    if let Some(info) = player_registry.registry.get(&player_id) {
        let _ = sim_tx.0.send(crate::net::SimCommand::Input {
            entity_id: info.entity_id,
            dx,
            dy,
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

