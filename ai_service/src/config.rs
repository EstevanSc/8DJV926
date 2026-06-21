use bevy::prelude::Resource;
use std::env;

/// Configuration for the AI Service loaded from environment variables.
#[derive(Resource, Clone, Debug)]
pub struct Config {
    pub broker_host: String,
    pub broker_port: u16,
    pub spawn_frequency_secs: f32,
    pub max_ai: usize,
    pub spawn_top_shard_percentage: f32,
    pub spawn_padding: f32,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();

        Config {
            broker_host: env::var("AI_SERVICE_BROKER_HOST")
                .unwrap_or_else(|_| "localhost".to_string()),
            broker_port: env::var("AI_SERVICE_BROKER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(7776),
            spawn_frequency_secs: env::var("AI_SERVICE_SPAWN_FREQUENCY_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5.0),
            max_ai: env::var("AI_SERVICE_MAX_AI")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            spawn_top_shard_percentage: env::var("AI_SERVICE_SPAWN_TOP_SHARD_PERCENTAGE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20.0),
            spawn_padding: env::var("AI_SERVICE_SPAWN_PADDING")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
        }
    }
}
