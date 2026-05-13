//! Configuration management from environment variables.
//! Loads PORT, ORCH_PORT, ENVIRONMENT, and REDIS_URL from .env file or environment variables.
//! Falls back to sensible defaults if neither .env nor environment variables are set.

use std::env;

const DEFAULT_PORT: u16 = 8081;
const DEFAULT_ORCH_PORT: u16 = 7000;
const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";
const DEFAULT_ENVIRONMENT: &str = "development";

pub struct Config {
    pub port: u16,
    pub orch_port: u16,
    pub redis_url: String,
    pub environment: String,
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

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| DEFAULT_REDIS_URL.to_string());

        let environment =
            env::var("ENVIRONMENT").unwrap_or_else(|_| DEFAULT_ENVIRONMENT.to_string());

        Config {
            port,
            orch_port,
            redis_url,
            environment,
        }
    }
}
