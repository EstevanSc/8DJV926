//! Configuration management from environment variables.
//! Loads PORT, ORCH_PORT, ENVIRONMENT, and REDIS_URL from .env file or environment variables.
//! Falls back to sensible defaults if neither .env nor environment variables are set.

use std::env;

const DEFAULT_PORT: u16 = 8081;
const DEFAULT_ORCH_PORT: u16 = 7000;
const DEFAULT_QUIC_PORT: u16 = 5000;
const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";
const DEFAULT_ENVIRONMENT: &str = "development";
const DEFAULT_DS_BASE_PORT: u16 = 7777;
const DEFAULT_HEARTBEAT_TTL_SECONDS: usize = 30;

pub struct Config {
    pub port: u16,
    pub orch_port: u16,
    pub quic_port: u16,
    pub redis_url: String,
    pub environment: String,
    pub ds_base_port: u16,
    pub heartbeat_ttl_seconds: usize,
}

impl Config {
    /// Loads configuration from .env file, environment variables, or defaults.
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();

        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);

        let orch_port = env::var("ORCH_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_ORCH_PORT);

        let quic_port = env::var("ORCHESTRATOR_QUIC_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_QUIC_PORT);

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| DEFAULT_REDIS_URL.to_string());

        let environment =
            env::var("ENVIRONMENT").unwrap_or_else(|_| DEFAULT_ENVIRONMENT.to_string());

        let ds_base_port = env::var("DS_BASE_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_DS_BASE_PORT);

        let heartbeat_ttl_seconds = env::var("HEARTBEAT_TTL_SECONDS")
            .ok()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(DEFAULT_HEARTBEAT_TTL_SECONDS);

        Config {
            port,
            orch_port,
            quic_port,
            redis_url,
            environment,
            ds_base_port,
            heartbeat_ttl_seconds,
        }
    }
}
