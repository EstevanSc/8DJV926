pub mod input;
pub mod interpolation;
pub mod login;
pub mod net;

use bevy::prelude::*;
use rustls;


use input::ClientInputPlugin;
use login::LoginPlugin;
use net::ClientNetPlugin;
use interpolation::InterpolationPlugin;


/// The game session received after a successful `/login`.
#[derive(Resource, Clone)]
#[allow(dead_code)] // server_ip / server_port used when real QUIC connection is implemented
pub struct GameSession {
    pub player_id: String,
    pub username: String,
    pub server_ip: String,
    pub server_port: u16,
    pub server_zone: String,
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
        .add_plugins(DefaultPlugins.set(WindowPlugin {
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
