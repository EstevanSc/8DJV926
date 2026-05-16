//! UDP heartbeat listener for server status updates.

use crate::infrastructure::RedisClient;
use crate::models::Heartbeat;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Starts the UDP heartbeat listener on the configured port.
///
/// Continuously listens for incoming heartbeat packets from Dedicated Servers,
/// parses JSON payloads, and persists server state to Redis.
pub async fn start_heartbeat_listener(port: u16, redis: RedisClient, heartbeat_ttl_seconds: usize) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    match UdpSocket::bind(addr).await {
        Ok(socket) => {
            tracing::info!("Heartbeat listener started on {}", addr);
            listen_loop(socket, redis, heartbeat_ttl_seconds).await;
        }
        Err(e) => {
            tracing::error!("Failed to bind heartbeat listener on {}: {}", addr, e);
        }
    }
}

/// Continuously receives UDP packets, parses them, and persists to Redis.
async fn listen_loop(socket: UdpSocket, redis: RedisClient, heartbeat_ttl_seconds: usize) {
    let mut buffer = vec![0u8; 65535];

    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((n, addr)) => {
                let payload = &buffer[..n];
                tracing::info!("Received heartbeat from {}: {} bytes", addr, n);

                // Parse JSON payload
                match String::from_utf8(payload.to_vec()) {
                    Ok(json_str) => {
                        let json_start = json_str.find('{').unwrap_or(0);
                        let json_content = &json_str[json_start..];
                        match serde_json::from_str::<Heartbeat>(json_content) {
                            Ok(heartbeat) => {
                                tracing::debug!(
                                    "Parsed heartbeat: id={}, zone={}, players={}/{}",
                                    heartbeat.id,
                                    heartbeat.zone,
                                    heartbeat.player_count,
                                    heartbeat.max_players
                                );

                                // Determine server status: empty (0 players), full, or available
                                let status = if heartbeat.player_count == 0 {
                                    "empty".to_string()
                                } else if heartbeat.player_count >= heartbeat.max_players {
                                    "full".to_string()
                                } else {
                                    "available".to_string()
                                };

                                // Prepare server data
                                let mut fields = HashMap::new();
                                fields.insert("ip", heartbeat.ip.clone());
                                fields.insert("port", heartbeat.port.to_string());
                                fields.insert("zone", heartbeat.zone.clone());
                                fields.insert("player_count", heartbeat.player_count.to_string());
                                fields.insert("max_players", heartbeat.max_players.to_string());
                                fields.insert("status", status.clone());

                                // Persist to Redis
                                let redis_key = format!("server:{}", heartbeat.id);
                                match redis.hset_multiple(&redis_key, fields).await {
                                    Ok(()) => {
                                        // Set TTL for automatic expiration
                                        if let Err(e) =
                                            redis.expire(&redis_key, heartbeat_ttl_seconds).await
                                        {
                                            tracing::error!(
                                                "Failed to set TTL for {}: {}",
                                                redis_key,
                                                e
                                            );
                                        } else {
                                            tracing::info!(
                                                "Updated server {} in Redis (status: {}, TTL: {}s)",
                                                heartbeat.id,
                                                status,
                                                heartbeat_ttl_seconds
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to persist server {} to Redis: {}",
                                            heartbeat.id,
                                            e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Invalid heartbeat JSON from {}: {}", addr, e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse heartbeat payload as UTF-8 from {}: {}",
                            addr,
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("Error receiving heartbeat: {}", e);
            }
        }
    }
}
