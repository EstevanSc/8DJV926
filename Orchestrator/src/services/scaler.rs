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

                    for _ in 0..needed {
                        match find_available_port(&redis, ds_base_port).await {
                            Ok(port) => {
                                spawn_dedicated_server(&ds_binary_path, port);
                            }
                            Err(e) => {
                                tracing::error!("Scaler: Failed to find available port: {}", e);
                            }
                        }
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

/// Scans Redis for all occupied ports from existing servers.
async fn get_occupied_ports(redis: &RedisClient) -> Result<Vec<u16>, redis::RedisError> {
    let keys = redis.scan("server:*").await?;
    let mut ports = Vec::new();

    for key in keys {
        if let Ok(Some(port_str)) = redis.hget(&key, "port").await {
            if let Ok(port) = port_str.parse::<u16>() {
                ports.push(port);
            }
        }
    }

    ports.sort();
    Ok(ports)
}

/// Finds the first available port starting from base_port, avoiding collisions.
async fn find_available_port(
    redis: &RedisClient,
    base_port: u16,
) -> Result<u16, redis::RedisError> {
    let occupied = get_occupied_ports(redis).await?;
    let mut candidate = base_port;

    while occupied.contains(&candidate) {
        candidate = candidate.saturating_add(1);
        if candidate > 65535u16.saturating_sub(1000) {
            candidate = base_port;
            break;
        }
    }

    Ok(candidate)
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
