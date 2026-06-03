//! Shard handler service for responding to quadtree shard layout updates.
//!
//! Receives shard update messages from quadtree, compares with current server
//! layout in Redis, and spawns/destroys game servers as needed.

use bollard::Docker;
use common::{Boundary, RedisClient};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

use crate::docker_ops;
use crate::quic_server::ShardUpdateMessage;
type CenterKey = (u64, u64);

fn center_key(boundary: &Boundary) -> CenterKey {
    (boundary.x.to_bits(), boundary.y.to_bits())
}

/// Handle shard updates from quadtree and manage game server lifecycle.
pub async fn start_shard_handler(
    docker: Docker,
    redis: RedisClient,
    base_port: u16,
    mut rx: mpsc::Receiver<ShardUpdateMessage>,
) {
    let mut current_shards: HashMap<CenterKey, String> = HashMap::new(); // center -> redis_key
    let mut current_centers: HashSet<CenterKey> = HashSet::new();

    while let Some(message) = rx.recv().await {
        if let Err(e) = handle_shard_update(
            &docker,
            &redis,
            &message.boundaries,
            &mut current_shards,
            &mut current_centers,
            base_port,
        )
        .await
        {
            tracing::error!("Failed to handle shard update: {}", e);
        }
    }

    tracing::error!("Shard handler channel closed");
}

/// Process a shard update and spawn/destroy servers as needed.
async fn handle_shard_update(
    docker: &Docker,
    redis: &RedisClient,
    boundaries: &[Boundary],
    current_shards: &mut HashMap<CenterKey, String>,
    current_centers: &mut HashSet<CenterKey>,
    base_port: u16,
) -> anyhow::Result<()> {
    let incoming_centers: HashSet<CenterKey> = boundaries.iter().map(center_key).collect();

    if *current_centers == incoming_centers {
        tracing::debug!("Orchestrator: shard boundary layout unchanged");
        return Ok(());
    }

    let to_remove: Vec<CenterKey> = current_shards
        .iter()
        .filter_map(|(center, _)| {
            if incoming_centers.contains(center) { None } else { Some(*center) }
        })
        .collect();

    let to_add: Vec<Boundary> = boundaries
        .iter()
        .copied()
        .filter(|boundary| !current_centers.contains(&center_key(boundary)))
        .collect();

    let to_add_len = to_add.len();
    let to_remove_len = to_remove.len();

    for center in to_remove {
        if let Some(redis_key) = current_shards.remove(&center) {
            current_centers.remove(&center);
            tracing::info!("Orchestrator: stopping server for shard center ({}, {})", f64::from_bits(center.0), f64::from_bits(center.1));

            if let Ok(Some(container_id)) = redis.hget(&redis_key, "container_id").await {
                if let Err(e) = docker_ops::stop_container(docker, &container_id).await {
                    tracing::error!(
                        "Failed to stop container for shard center ({}, {}): {} — leaving Redis entry intact",
                        f64::from_bits(center.0),
                        f64::from_bits(center.1),
                        e
                    );
                    continue;
                }
            }

            if let Err(e) = redis.del(&redis_key).await {
                tracing::error!("Failed to remove Redis key {}: {}", redis_key, e);
            } else {
                tracing::info!("Removed server for shard center ({}, {})", f64::from_bits(center.0), f64::from_bits(center.1));
            }
        }
    }

    for boundary in to_add {
        let center = center_key(&boundary);
        tracing::info!("Orchestrator: spawning server for shard center ({}, {})", boundary.x, boundary.y);

        match find_available_port(redis, base_port).await {
            Ok(port) => {
                match docker_ops::spawn_server_for_boundary(docker, redis, port, boundary).await {
                    Ok(redis_key) => {
                        current_shards.insert(center, redis_key);
                        current_centers.insert(center);
                    }
                    Err(e) => {
                        tracing::error!("Failed to spawn server for shard center ({}, {}): {}", boundary.x, boundary.y, e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("No available port for shard center ({}, {}): {}", boundary.x, boundary.y, e);
            }
        }
    }

    tracing::info!(
        "Orchestrator: shard layout updated — active shards: {} (added: {}, removed: {})",
        current_shards.len(),
        to_add_len,
        to_remove_len
    );

    Ok(())
}

/// Find an available port for a new game server.
async fn find_available_port(redis: &RedisClient, base_port: u16) -> anyhow::Result<u16> {
    let keys = redis.scan("server:*").await?;
    let mut used_ports = HashSet::new();

    for key in keys {
        if let Ok(Some(port_str)) = redis.hget(&key, "port").await {
            if let Ok(port) = port_str.parse::<u16>() {
                used_ports.insert(port);
            }
        }
    }

    for port in base_port..base_port + 1000 {
        if !used_ports.contains(&port) {
            return Ok(port);
        }
    }

    Err(anyhow::anyhow!("No available port in range {}-{}", base_port, base_port + 1000))
}
