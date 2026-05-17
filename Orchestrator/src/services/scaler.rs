//! Server scaling loop that maintains a minimum fleet of empty game servers.
//!
//! Each tick the scaler:
//!   - counts servers with `status=empty` in Redis
//!   - **spawns** Docker containers if the count is below `hot_servers_min`
//!   - **stops** excess containers if the count is above `hot_servers_min`

use bollard::Docker;
use common::RedisClient;
use tokio::time::{Duration, interval};

use crate::docker_ops;

/// Main scaler loop. Runs forever; call from a `tokio::spawn`.
pub async fn start_scaler(
    docker: Docker,
    redis: RedisClient,
    hot_servers_min: usize,
    base_port: u16,
    scaler_interval_seconds: u64,
) {
    let mut ticker = interval(Duration::from_secs(scaler_interval_seconds));

    loop {
        ticker.tick().await;

        match list_empty_servers(&redis).await {
            Ok(empty_servers) => {
                let empty_count = empty_servers.len();
                tracing::debug!("Scaler: {empty_count} empty server(s) (min={hot_servers_min})");

                if empty_count < hot_servers_min {
                    // ── scale up ─────────────────────────────────────────────
                    let needed = hot_servers_min - empty_count;
                    tracing::info!("Scaler: spawning {needed} server(s)");

                    for _ in 0..needed {
                        match find_available_port(&redis, base_port).await {
                            Ok(port) => {
                                if let Err(e) =
                                    docker_ops::spawn_server(&docker, &redis, port).await
                                {
                                    tracing::error!("Scaler: failed to spawn server: {e}");
                                }
                            }
                            Err(e) => {
                                tracing::error!("Scaler: no free port available: {e}");
                            }
                        }
                    }
                } else if empty_count > hot_servers_min {
                    // ── scale down ────────────────────────────────────────────
                    let excess = empty_count - hot_servers_min;
                    tracing::info!("Scaler: removing {excess} excess empty server(s)");

                    for (redis_key, container_id) in empty_servers.into_iter().take(excess) {
                        // Stop the Docker container if we know its ID.
                        if let Some(cid) = container_id {
                            if let Err(e) = docker_ops::stop_container(&docker, &cid).await {
                                tracing::error!(
                                    "Scaler: failed to stop container {cid}: {e} \
                                     — leaving Redis entry intact"
                                );
                                continue; // don't remove the Redis key if the container is still up
                            }
                        }

                        // Remove the Redis entry.
                        if let Err(e) = redis.del(&redis_key).await {
                            tracing::error!("Scaler: failed to remove Redis key {redis_key}: {e}");
                        } else {
                            tracing::info!("Scaler: removed server {redis_key}");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!("Scaler: failed to query Redis: {e}");
            }
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Returns `(redis_key, container_id)` for every server with `status=empty`.
async fn list_empty_servers(redis: &RedisClient) -> anyhow::Result<Vec<(String, Option<String>)>> {
    let keys = redis.scan("server:*").await?;
    let mut result = Vec::new();

    for key in keys {
        if let Ok(Some(status)) = redis.hget(&key, "status").await {
            if status == "empty" {
                let container_id = redis.hget(&key, "container_id").await.unwrap_or(None);
                result.push((key, container_id));
            }
        }
    }

    Ok(result)
}

/// Returns ports already registered in Redis under any `server:*` key.
async fn get_occupied_ports(redis: &RedisClient) -> anyhow::Result<Vec<u16>> {
    let keys = redis.scan("server:*").await?;
    let mut ports = Vec::new();

    for key in keys {
        if let Ok(Some(port_str)) = redis.hget(&key, "port").await {
            if let Ok(port) = port_str.parse::<u16>() {
                ports.push(port);
            }
        }
    }

    ports.sort_unstable();
    Ok(ports)
}

/// Finds the lowest port ≥ `base_port` that is not already in use.
async fn find_available_port(redis: &RedisClient, base_port: u16) -> anyhow::Result<u16> {
    let occupied = get_occupied_ports(redis).await?;
    let mut candidate = base_port;

    while occupied.contains(&candidate) {
        candidate = candidate.saturating_add(1);
        if candidate == u16::MAX {
            candidate = base_port;
            break;
        }
    }

    Ok(candidate)
}
