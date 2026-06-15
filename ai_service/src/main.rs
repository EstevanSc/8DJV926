mod behaviour;
mod bridge;
mod client;
mod components;
mod config;
mod spawn_manager;

use std::sync::Arc;

use bevy::log::LogPlugin;
use bevy::prelude::*;
use tokio::runtime::Runtime;

use bridge::{BridgePlugin, TokioRuntime};
use config::Config;
use spawn_manager::SpawnPlugin;
use behaviour::BehaviourPlugin;

fn main() {
    let config = Config::from_env();
    let runtime = Arc::new(Runtime::new().expect("Failed to build tokio runtime"));

    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(LogPlugin::default())
        .insert_resource(config)
        .insert_resource(TokioRuntime(runtime))
        .add_plugins(BridgePlugin)
        .add_plugins(SpawnPlugin)
        .add_plugins(BehaviourPlugin)
        .run();
}
