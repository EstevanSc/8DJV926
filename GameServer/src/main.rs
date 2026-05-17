mod heartbeat;
mod messages;
mod server;

use crate::server::ServerPlugin;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(ServerPlugin)
        .run();
}
