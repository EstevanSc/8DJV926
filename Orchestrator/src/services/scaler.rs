//! Server scaling loop that maintains minimum available servers.

use crate::infrastructure::RedisClient;
use std::process::Command;
use tokio::time::{interval, Duration};

const SCALER_INTERVAL_SECONDS: u64 = 5;

/// Starts the scaler loop
/// Scans available servers and spawns new dedicated server processes if needed.
pub async fn start_scaler(
    redis: RedisClient,
    hot_servers_min: usize,
    ds_binary_path: String,
    ds_base_port: u16,
) {
    let mut interval = interval(Duration::from_secs(SCALER_INTERVAL_SECONDS));

    loop {
        interval.tick().await;

        match scan_available_servers(&redis).await {
            Ok(available_count) => {
                tracing::debug!("Scaler: {} available servers", available_count);

                if available_count < hot_servers_min {
                    let needed = hot_servers_min - available_count;
                    tracing::info!("Scaler: Need to spawn {} servers", needed);

                    for i in 0..needed {
                        let port = ds_base_port + (available_count as u16) + (i as u16);
                        spawn_dedicated_server(&ds_binary_path, port);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Scaler: Failed to scan servers: {}", e);
            }
        }
    }
}

/// Scans Redis for all servers matching "server:*" and counts available ones.
async fn scan_available_servers(redis: &RedisClient) -> Result<usize, redis::RedisError> {
    let keys = redis.scan("server:*").await?;

    let mut available_count = 0;

    for key in keys {
        if let Ok(Some(status)) = redis.hget(&key, "status").await {
            if status == "available" {
                available_count += 1;
            }
        }
    }

    Ok(available_count)
}

/// Spawns a new dedicated server process using the provided binary path and port.
fn spawn_dedicated_server(binary_path: &str, port: u16) {
    match Command::new(binary_path).arg(port.to_string()).spawn() {
        Ok(child) => {
            tracing::info!(
                "Spawned dedicated server on port {} (PID: {:?})",
                port,
                child.id()
            );
        }
        Err(e) => {
            tracing::error!(
                "Failed to spawn dedicated server at {} on port {}: {}",
                binary_path,
                port,
                e
            );
        }
    }
}
