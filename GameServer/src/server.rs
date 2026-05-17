use crate::heartbeat::Heartbeat;
use crate::messages::GameMessage;
use bevy::prelude::*;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameNetworkEvent, GamePeer};
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
            .add_systems(Startup, bind_socket)
            .add_systems(Update, (receive_packets, send_heartbeat).chain());
    }
}

#[derive(Resource, Default)]
pub struct PlayerRegistry {
    pub registry: HashMap<Uuid, PlayerInfo>,
}

pub struct PlayerInfo {
    pub id: Uuid,
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

fn receive_packets(mut server: ResMut<NetworkPeer>, mut player_registry: ResMut<PlayerRegistry>) {
    while let Ok(Some(event)) = server.peer.poll() {
        match event {
            GameNetworkEvent::Connected(conn) => {
                println!("Connected! Client id: {:?}", conn.connection_id);
            }
            GameNetworkEvent::Message {
                data,
                connection,
                stream,
            } => {
                let msg: GameMessage = wincode::deserialize(&data).unwrap();
                match msg {
                    // JOIN message
                    GameMessage::Join { username } => {
                        println!("Joined {}", username);
                        let id = connection.connection_id;
                        player_registry
                            .registry
                            .insert(id, PlayerInfo { id, username });

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
            }
            GameNetworkEvent::Disconnected(conn) => {
                // Remove player from registry
                player_registry.registry.remove(&conn.connection_id);
                println!("Disconnected! Client id: {:?}", conn.connection_id);
            }
            _ => {}
        }
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
            max_players: config.max_players.clone(),
        };

        // Send heartbeat JSON packet to the orchestrator
        if let Ok(json_payload) = serde_json::to_string(&heartbeat_data) {
            if let Ok(udp_socket) = UdpSocket::bind("0.0.0.0:0") {
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
        }
    }
}
