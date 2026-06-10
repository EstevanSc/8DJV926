pub mod input;
pub mod interpolation;
pub mod login;
pub mod net;

use bevy::prelude::*;
use bevy::log::LogPlugin;

use self::input::ClientInputPlugin;
use self::interpolation::InterpolationPlugin;
use self::login::LoginPlugin;
use self::net::ClientNetPlugin;

/// The game session received after a successful `/login`.
#[derive(Resource, Clone)]
#[allow(dead_code)] // server_ip / server_port used when real QUIC connection is implemented
pub struct GameSession {
    pub player_id: String,
    pub username: String,
    pub broker_ip: String,
    pub broker_port: u16,
}

/// Top-level app states.
#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    #[default]
    Login,
    Connecting,
    InGame,
}

pub fn run() {
    // Install the ring crypto provider as the process-level default for rustls.
    // Required when multiple providers are available (ring + aws-lc-rs via reqwest).
    let _ = rustls::crypto::ring::default_provider().install_default();

    App::new()
        .add_plugins(DefaultPlugins
            .set(LogPlugin {
                filter: "info,client=debug".to_string(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Extraction MMO".to_string(),
                    resolution: bevy::window::WindowResolution::new(1280_u32, 720_u32),
                    ..default()
                }),
                ..default()
            }))
        .init_state::<GameState>()
        .add_plugins(LoginPlugin)
        .add_plugins(ClientNetPlugin)
        .add_plugins(ClientInputPlugin)
        .add_plugins(InterpolationPlugin)
        .run();
}
