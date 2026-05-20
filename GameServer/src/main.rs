mod char_controller;
mod heartbeat;
mod interest;
mod messages;
mod net;
mod server;
mod simulation;

use std::sync::Mutex;

use bevy::prelude::*;

use crate::net::{ConnectedPlayers, SimCommandReceiver, SimCommandSender};
use crate::server::ServerPlugin;
use crate::simulation::SimulationPlugin;

fn main() {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<net::SimCommand>();

    App::new()
        .add_plugins(MinimalPlugins)
        .insert_resource(SimCommandSender(cmd_tx))
        .insert_resource(SimCommandReceiver(Mutex::new(cmd_rx)))
        .insert_resource(ConnectedPlayers::default())
        .add_plugins(ServerPlugin)
        .add_plugins(SimulationPlugin)
        .run();
}
