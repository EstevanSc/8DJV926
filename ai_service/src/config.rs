use bevy::prelude::Resource;
use serde::Deserialize;

/// AI kind — one variant per NPC type.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum AiKind {
    Goblin,
}

/// A single spawn zone loaded from config.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct ZoneConfig {
    pub kind: AiKind,
    pub center: [f64; 2],
    pub count: usize,
    pub respawn_delay_secs: u64,
}

/// Root configuration deserialized from config.toml.
#[derive(Debug, Clone, Deserialize, Resource)]
pub struct Config {
    pub broker_ip: String,
    pub broker_port: u16,
    pub zones: Vec<ZoneConfig>,
}

impl Config {
    /// Load and parse `config.toml` from the current working directory.
    pub fn load() -> Self {
        let raw = std::fs::read_to_string("config.toml")
            .expect("config.toml not found");
        toml::from_str(&raw).expect("Invalid config.toml")
    }
}