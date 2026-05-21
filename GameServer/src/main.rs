mod authority;
mod char_controller;
mod heartbeat;
mod interest;
mod messages;
mod net;
mod server;
mod simulation;

use std::sync::Mutex;

use bevy::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::authority::AuthorityPlugin;
use crate::net::{ConnectedPlayers, SimCommandReceiver, SimCommandSender};
use crate::server::ServerPlugin;
use crate::simulation::SimulationPlugin;

fn main() {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .map(EnvFilter::new)
        .unwrap_or_else(|| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<net::SimCommand>();

    App::new()
        .add_plugins(MinimalPlugins)
        .insert_resource(SimCommandSender(cmd_tx))
        .insert_resource(SimCommandReceiver(Mutex::new(cmd_rx)))
        .insert_resource(ConnectedPlayers::default())
        .add_plugins(ServerPlugin)
        .add_plugins(AuthorityPlugin)
        .add_plugins(SimulationPlugin)
        .run();
}
