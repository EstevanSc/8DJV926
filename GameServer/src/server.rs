use crate::heartbeat::Heartbeat;
use crate::net::{ConnectedPlayers, SimCommandSender};
use common::broker_messages::{BrokerMessage, SendingSystem};
use common::topics::{
   PositionPayload, ShardCreatedPayload, Topic, deserialize_input_payload, deserialize_position_payload, serialize_position_payload, serialize_shard_created_payload, AuthorityDebugPacketPayload, serialize_authority_debug_packet_payload
};
use common::{Boundary};
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
    pub registry: HashMap<Uuid, ClientInfo>,
}


#[allow(dead_code)]
pub struct ClientInfo {
    pub is_ghost: bool,
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
    pub shard_boundary: Boundary,
    pub shard_margin: f32,
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

        let shard_x = std::env::var("DS_SHARD_CENTER_X")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let shard_y = std::env::var("DS_SHARD_CENTER_Y")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0);
        let shard_half_size = std::env::var("DS_SHARD_HALF_SIZE")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(100.0);



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
            shard_margin: std::env::var("QUADTREE_NEARBY_MARGIN")
                .unwrap_or_else(|_| "100.0".to_string())
                .parse::<f32>()
                .unwrap(),
            orchestrator_address,
            shard_boundary: Boundary {
                x: shard_x,
                y: shard_y,
                half_size: shard_half_size,
            },
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
    server_config: Res<ServerConfig>,
    mut client_registry: ResMut<PlayerRegistry>,
    sim_tx: Res<SimCommandSender>,
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
                    Some(&mut *broker),
                    &mut client_registry,
                    &sim_tx,
                    &server_config,
                );
            }
            GameNetworkEvent::StreamCreated(connection, stream) => {
                if broker.connection == Some(connection) {
                    if stream.is_reliable() && broker.control_stream.is_none() {
                        broker.control_stream = Some(stream);

                        subscribe_ownership_topics(&mut broker);
                        
                        try_announce_shard_creation(&mut broker, &server_config);
                    }

                }
            }
            GameNetworkEvent::Disconnected(connection) => {
                eprintln!(
                    "Broker disconnected! Connection id: {:?}",
                    connection.connection_id
                );
                broker.connection = None;
                broker.control_stream = None;
            }
            GameNetworkEvent::Error { inner, .. } => {
                eprintln!("Broker network error: {:?}", inner);
            }
            _ => {}
        }
    }
}

fn subscribe_ownership_topics(broker: &mut ResMut<BrokerPeer>) {
    let (Some(connection), Some(control_stream)) = (
        broker.connection,
        broker.control_stream.clone(),
    ) else {
        return;
    };

    for topic in [
        Topic::ClaimOwnership(connection.connection_id),
        Topic::ReleaseOwnership(connection.connection_id),
    ] {
        let subscribe_message =
            BrokerMessage::serialize_subscribe(connection.connection_id, topic.to_bytes());

        if let Err(e) = broker
            .peer
            .send(&connection, &control_stream, subscribe_message.into())
        {
            eprintln!("Failed to subscribe to ownership topic: {:?}", e);
        }
    }

    println!(
        "Subscribed to ownership topics for client_id={}",
        connection.connection_id
    );
}

fn receive_packets(
    mut server: ResMut<NetworkPeer>,
    mut client_registry: ResMut<PlayerRegistry>,
    sim_tx: Res<SimCommandSender>,
    conn_players: ResMut<ConnectedPlayers>,
    server_config: Res<ServerConfig>,
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
                    None,
                    &mut client_registry,
                    &sim_tx,
                    &server_config,
                );
            }
            GameNetworkEvent::Disconnected(conn) => {
                if let Ok(mut map) = conn_players.0.lock() {
                    map.remove(&conn.connection_id);
                }

                client_registry.registry.remove(&conn.connection_id);

                let _ = sim_tx.0.send(crate::net::SimCommand::Left {
                    connection_id: conn.connection_id,
                });
                
                println!("Disconnected! Client id: {:?}", conn.connection_id);
            }
            _ => {}
        }
    }
}

fn try_announce_shard_creation(broker: &mut ResMut<BrokerPeer>, server_config: &Res<ServerConfig>) {
    let (Some(connection), Some(control_stream)) = (
        broker.connection,
        broker.control_stream.clone(),
    ) else {
        return;
    };

    let connect_message = BrokerMessage::serialize_connect(connection.connection_id, SendingSystem::Server);
    if let Err(e) = broker.peer.send(&connection, &control_stream, connect_message.into()) {
        eprintln!("Failed to send broker Connect message: {:?}", e);
        return;
    }

    let shard_created_payload = ShardCreatedPayload {
        shard_connection_id: connection.connection_id,
        boundary: server_config.shard_boundary,
    };
    
    let publish_message = BrokerMessage::serialize_publish(
        Topic::ShardCreated.to_bytes(),
        &serialize_shard_created_payload(&shard_created_payload),
    );

    if let Err(e) = broker.peer.send(&connection, &control_stream, publish_message.into()) {
        eprintln!("Failed to send broker ShardCreated publish: {:?}", e);
        return;
    }

    println!("Announced shard creation to broker with shard_connection_id={}", connection.connection_id);
}

fn handle_message(
    data: &[u8],
    connection: game_sockets::GameConnection,
    stream: GameStream,
    mut broker: Option<&mut BrokerPeer>,
    client_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
    server_config: &Res<ServerConfig>,
) {
    if let Some(message) = BrokerMessage::deserialize(data) {
        handle_broker_message(
            message,
            connection,
            stream,
            broker.as_deref_mut(),
            client_registry,
            sim_tx,   
            server_config,
        );
        return;
    }
}

fn handle_broker_message(
    message: BrokerMessage,
    connection: game_sockets::GameConnection,
    _stream: GameStream,
    broker: Option<&mut BrokerPeer>,
    client_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
    server_config: &Res<ServerConfig>,
) {
    match message {
        BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
            Topic::PlayerStartingPositionInShard(client_id) => {
                if let Some(position_payload) = deserialize_position_payload(&payload) {
                    handle_receive_new_player(
                        client_id,
                        position_payload.position[0] as f32,
                        position_payload.position[1] as f32,
                        broker,
                        client_registry,
                        sim_tx,
                    );
                } else {
                    eprintln!("Failed to decode broker PlayerStartingPositionInShard payload from {:?}", connection.connection_id);
                }
            }
            Topic::EntityPositionUpdate(entity_id) => {
                let Some(position_payload) = deserialize_position_payload(&payload) else {
                    eprintln!("Failed to decode EntityPositionUpdate payload for entity_id={}", entity_id);
                    return;
                };

                match client_registry.registry.get(&entity_id) {
                    Some(info) if info.is_ghost => {
                        handle_ghost_position_update(
                            entity_id,
                            position_payload.position[0] as f32,
                            position_payload.position[1] as f32,
                            sim_tx,
                            server_config,
                        );
                    }
                    Some(_) => {
                        trace!("Ignoring broker position update for locally owned entity_id={}", entity_id);
                    }
                    None => {
                        handle_ghost_joined(
                            entity_id,
                            entity_id,
                            position_payload.position[0] as f32,
                            position_payload.position[1] as f32,
                            client_registry,
                            sim_tx,
                        );
                    }
                }
            }
            Topic::Input(client_id) => {
                trace!("Received Input publish from client_id={}", client_id);
                if let Some(input) = deserialize_input_payload(&payload) {
                    trace!("Received input from client_id={}: dx={}, dy={}", client_id, input.dxdy[0], input.dxdy[1]);
                    handle_player_input(
                        client_id,
                        input.dxdy[0] as f32,
                        input.dxdy[1] as f32,
                        client_registry,
                        sim_tx,
                    );
                } else {
                    eprintln!("Failed to decode broker input payload from {:?}", connection.connection_id);
                }
            }
            Topic::Disconnect(client_id) => {
                trace!("Received disconnect broadcast for client_id={}", client_id);
                client_registry.registry.remove(&client_id);
                let _ = sim_tx.0.send(crate::net::SimCommand::Left {
                    connection_id: client_id,
                });
            }
            Topic::ClaimOwnership(shard_id) => {
                let Ok(connection_id) = Uuid::from_slice(&payload) else {
                    eprintln!("Received ClaimOwnership for shard_id={} with invalid payload", shard_id);
                    return;
                };

                trace!("Received ClaimOwnership broadcast for shard_id={} connection_id={}", shard_id, connection_id);
                if let Some(info) = client_registry.registry.get_mut(&connection_id) {
                    info.is_ghost = false;
                }
                let _ = sim_tx.0.send(crate::net::SimCommand::GhostIsNowLocal {
                    connection_id,
                });
            }
            Topic::ReleaseOwnership(shard_id) => {
                let Ok(connection_id) = Uuid::from_slice(&payload) else {
                    eprintln!("Received ReleaseOwnership for shard_id={} with invalid payload", shard_id);
                    return;
                };

                trace!("Received ReleaseOwnership broadcast for shard_id={} connection_id={}", shard_id, connection_id);
                if let Some(info) = client_registry.registry.get_mut(&connection_id) {
                    info.is_ghost = true;
                }
                let _ = sim_tx.0.send(crate::net::SimCommand::LocalIsNowGhost {
                    connection_id,
                });
            }
            _ => {}
        },
        _ => {}
    }
}

pub(crate) fn publish_player_position(broker: &BrokerPeer, connection_id: Uuid ,position_payload: PositionPayload) {
    let (Some(connection), Some(control_stream)) = (broker.connection, broker.control_stream.clone()) else {
        return;
    };

    let payload_bytes = serialize_position_payload(&position_payload); 

    let publish_message = BrokerMessage::serialize_publish(
        Topic::EntityPositionUpdate(connection_id).to_bytes(),
        &payload_bytes,
    );

    let debug_payload = serialize_authority_debug_packet_payload(&AuthorityDebugPacketPayload {
        sender_id: connection.connection_id,
    });

    let debug_message = BrokerMessage::serialize_publish(
        Topic::AuthorityDebugPacket(connection_id).to_bytes(),
        &debug_payload,
    );

    if let Err(e) = broker.peer.send(&connection, &control_stream, publish_message.into()) {
        eprintln!(
            "Failed to publish EntityPositionUpdate for client_id={}: {:?}",
            connection_id,
            e
        );
    }

    if let Err(e) = broker.peer.send(&connection, &control_stream, debug_message.into()) {
        eprintln!(
            "Failed to publish AuthorityDebugPacket for client_id={}: {:?}",
            connection_id,
            e
        );
    }
}

fn handle_player_input(
    client_id: Uuid,
    dx: f32,
    dy: f32,
    client_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
    trace!("Received input from client_id={}: dx={}, dy={}", client_id, dx, dy);
    if client_registry.registry.contains_key(&client_id) {
        let _ = sim_tx.0.send(crate::net::SimCommand::Input {
            connection_id: client_id,
            dx,
            dy,
        });
    }
}

fn handle_receive_new_player(
    client_id: Uuid,
    x: f32,
    y: f32,
    broker: Option<&mut BrokerPeer>,
    client_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
    if !client_registry.registry.contains_key(&client_id) {
        client_registry.registry.insert(
            client_id,
            ClientInfo { is_ghost: false },
        );

        let _ = sim_tx.0.send(crate::net::SimCommand::Joined {
            connection_id: client_id,
            position: Vec2 { x, y },
        });

        trace!("Registered new player with client_id={} at position=({}, {})", client_id, x, y);

        if let Some(broker) = broker {
            let (Some(connection), Some(control_stream)) = (broker.connection, broker.control_stream.clone())
            else {
                eprintln!(
                    "Failed to send initial PlayerStartingPosition publish for client_id={} because broker connection or control stream is missing",
                    client_id
                );
                return;
            };

            //subscribe to inputs from this client
            let topic = Topic::Input(client_id);
            let subscribe_message =
                BrokerMessage::serialize_subscribe(connection.connection_id, topic.to_bytes());
            if let Err(e) = broker
                .peer
                .send(&connection, &control_stream, subscribe_message.into())
            {
                eprintln!(
                    "Failed to subscribe client_id={} to Input topic: {:?}",
                    client_id, e
                );
            }

            let unsubscribe_spawn = BrokerMessage::serialize_unsubscribe(
                connection.connection_id,
                Topic::PlayerStartingPositionInShard(client_id).to_bytes(),
            );
            if let Err(e) = broker
                .peer
                .send(&connection, &control_stream, unsubscribe_spawn.into())
            {
                eprintln!(
                    "Failed to unsubscribe client_id={} from PlayerStartingPositionInShard: {:?}",
                    client_id, e
                );
            }
        }
    }
}

fn handle_ghost_joined(
    client_id: Uuid,
    connection_id: Uuid,
    x: f32,
    y: f32,
    client_registry: &mut ResMut<PlayerRegistry>,
    sim_tx: &Res<SimCommandSender>,
) {
    client_registry.registry.insert(
        client_id,
        ClientInfo { is_ghost: true },
    );

    let _ = sim_tx.0.send(crate::net::SimCommand::GhostJoined {
        connection_id,
        position: Vec2 { x, y },
    });
}

fn handle_ghost_position_update(
    connection_id: Uuid,
    x: f32,
    y: f32,
    sim_tx: &Res<SimCommandSender>,
    server_config: &Res<ServerConfig>,
) {
    let shard_boundary = &server_config.shard_boundary;
    let margin = server_config.shard_margin;
    //if the position update is outside the shard boundary + 2*margin, despawn the ghost
    if x < shard_boundary.x as f32 - shard_boundary.half_size as f32 - 2.0 * margin
        || x > shard_boundary.x as f32 + shard_boundary.half_size as f32 + 2.0 * margin
        || y < shard_boundary.y as f32 - shard_boundary.half_size as f32 - 2.0 * margin
        || y > shard_boundary.y as f32 + shard_boundary.half_size as f32 + 2.0 * margin
    {
        let _ = sim_tx.0.send(crate::net::SimCommand::Left {
            connection_id,
        });
        return;
    }

    let _ = sim_tx.0.send(crate::net::SimCommand::GhostPositionUpdate {
        connection_id,
        position: Vec2 { x, y },
    });
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

