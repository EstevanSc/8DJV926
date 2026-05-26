//! Shard handler service for responding to quadtree shard layout updates.
//!
//! Receives shard update messages from quadtree, compares with current server
//! layout in Redis, and spawns/destroys game servers as needed.

use bollard::Docker;
use common::{RedisClient, ShardData};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

use crate::docker_ops;
use crate::quic_server::ShardUpdateMessage;

/// Handle shard updates from quadtree and manage game server lifecycle.
pub async fn start_shard_handler(
    docker: Docker,
    redis: RedisClient,
    base_port: u16,
    mut rx: mpsc::Receiver<ShardUpdateMessage>,
) {
    let mut current_shards: HashMap<u32, String> = HashMap::new(); // shard_id -> redis_key

    while let Some(message) = rx.recv().await {
        if let Err(e) = handle_shard_update(
            &docker,
            &redis,
            &message.shard_data,
            &mut current_shards,
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
    shard_data: &[ShardData],
    current_shards: &mut HashMap<u32, String>,
    base_port: u16,
) -> anyhow::Result<()> {
    // Extract new shard IDs from the update
    let new_shard_ids: HashSet<u32> = shard_data
        .iter()
        .filter_map(|s| s.shard_id)
        .collect();

    // Find shards to add (new in update but not in current)
    let to_add: HashSet<u32> = new_shard_ids
        .iter()
        .copied()
        .filter(|id| !current_shards.contains_key(id))
        .collect();

    // Find shards to remove (in current but not in update)
    let to_remove: Vec<u32> = current_shards
        .keys()
        .copied()
        .filter(|id| !new_shard_ids.contains(id))
        .collect();

    let to_add_len = to_add.len();
    let to_remove_len = to_remove.len();

    // Spawn servers for new shards
    for shard_id in to_add {
        tracing::info!("Orchestrator: spawning server for shard {}", shard_id);

        match find_available_port(redis, base_port).await {
            Ok(port) => {
                match docker_ops::spawn_server_for_shard(docker, redis, port, shard_id).await {
                    Ok(redis_key) => {
                        current_shards.insert(shard_id, redis_key);
                    }
                    Err(e) => {
                        tracing::error!("Failed to spawn server for shard {}: {}", shard_id, e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("No available port for shard {}: {}", shard_id, e);
            }
        }
    }

    // Stop servers for removed shards
    for shard_id in to_remove {
        if let Some(redis_key) = current_shards.remove(&shard_id) {
            tracing::info!("Orchestrator: stopping server for shard {}", shard_id);

            if let Ok(Some(container_id)) = redis.hget(&redis_key, "container_id").await {
                if let Err(e) = docker_ops::stop_container(docker, &container_id).await {
                    tracing::error!(
                        "Failed to stop container for shard {}: {} — leaving Redis entry intact",
                        shard_id,
                        e
                    );
                    continue;
                }
            }

            if let Err(e) = redis.del(&redis_key).await {
                tracing::error!("Failed to remove Redis key {}: {}", redis_key, e);
            } else {
                tracing::info!("Removed server for shard {}", shard_id);
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
