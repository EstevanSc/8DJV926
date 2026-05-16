use bevy::asset::uuid;
use bevy::prelude::*;
use game_sockets::GamePeer;
use game_sockets::protocols::QuicBackend;
use uuid::Uuid;

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app
            .insert_resource(ServerConfig::from_env())
            .add_systems(Startup, bind_socket);
    }
}

#[derive(Resource)]
pub struct ServerConfig {
    pub id: String,
    pub ip: String,
    pub port: u16,
    pub zone: String,
    pub max_players: usize,
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let port = std::env::var("DS_PORT")
            .unwrap_or_else(|_| "9876".to_string())
            .parse::<u16>()
            .expect("Invalid DS_PORT");

        Self {
            id: Uuid::new_v4().to_string(),
            ip: "127.0.0.1".to_string(),
            port,
            zone: std::env::var("DS_ZONE").unwrap_or_else(|_| "zone_A".to_string()),
            max_players: std::env::var("MAX_PLAYERS")
                .unwrap_or_else(|_| "2".to_string()) // low number to test FULL states easily
                .parse::<usize>()
                .unwrap(),
        }
    }
}

#[derive(Resource)]
pub struct NetworkPeer {
    pub peer: GamePeer,
}

fn bind_socket(mut commands: Commands, server_config: Res<ServerConfig>) {
    let peer = GamePeer::new(QuicBackend::new());

    let ip = &server_config.ip;
    let port = server_config.port;

    match peer.listen(ip, port) {
        Ok(_) => {
            println!("Listening on {}", ip);
            commands.insert_resource(NetworkPeer {peer});
        }
        Err(e) => {
            eprintln!("Failed to listen on {}: {}", ip, e);
        }
    }
}