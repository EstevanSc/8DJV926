mod server;
mod messages;
mod heartbeat;

use bevy::prelude::*;
use crate::server::ServerPlugin;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(ServerPlugin)
        .run();
}
