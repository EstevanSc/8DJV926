//! Configuration management from environment variables.
//! Loads PORT, ENVIRONMENT, and REDIS_URL with sensible defaults.

use std::env;

pub struct Config {
    pub port: u16,
    pub redis_url: String,
    pub environment: String,
}

impl Config {
    /// Loads configuration from environment variables with defaults.
    pub fn from_env() -> Self {
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);

        let redis_url =
            env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

        let environment = env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string());

        Config {
            port,
            redis_url,
            environment,
        }
    }
}
