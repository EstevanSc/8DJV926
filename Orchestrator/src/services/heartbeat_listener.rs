//! UDP heartbeat listener for server status updates.

use crate::infrastructure::RedisClient;
use crate::models::Heartbeat;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::net::UdpSocket;

const HEARTBEAT_TTL_SECONDS: usize = 15;

/// Starts the UDP heartbeat listener on the configured port.
///
/// Continuously listens for incoming heartbeat packets from Dedicated Servers,
/// parses JSON payloads, and persists server state to Redis.
pub async fn start_heartbeat_listener(port: u16, redis: RedisClient) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    match UdpSocket::bind(addr).await {
        Ok(socket) => {
            tracing::info!("Heartbeat listener started on {}", addr);
            listen_loop(socket, redis).await;
        }
        Err(e) => {
            tracing::error!("Failed to bind heartbeat listener on {}: {}", addr, e);
        }
    }
}

/// Continuously receives UDP packets, parses them, and persists to Redis.
async fn listen_loop(socket: UdpSocket, mut redis: RedisClient) {
    let mut buffer = vec![0u8; 65535];

    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((n, addr)) => {
                let payload = &buffer[..n];
                tracing::info!("Received heartbeat from {}: {} bytes", addr, n);

                // Parse JSON payload
                match String::from_utf8(payload.to_vec()) {
                    Ok(json_str) => {
                        match serde_json::from_str::<Heartbeat>(&json_str) {
                            Ok(heartbeat) => {
                                tracing::debug!(
                                    "Parsed heartbeat: id={}, zone={}, players={}/{}",
                                    heartbeat.id,
                                    heartbeat.zone,
                                    heartbeat.player_count,
                                    heartbeat.max_players
                                );

                                // Determine server status
                                let status = if heartbeat.player_count >= heartbeat.max_players {
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
                                fields.insert("status", status);

                                // Persist to Redis
                                let redis_key = format!("server:{}", heartbeat.id);
                                match redis.hset_multiple(&redis_key, fields).await {
                                    Ok(()) => {
                                        // Set TTL for automatic expiration
                                        if let Err(e) =
                                            redis.expire(&redis_key, HEARTBEAT_TTL_SECONDS).await
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
                                                if heartbeat.player_count >= heartbeat.max_players {
                                                    "full"
                                                } else {
                                                    "available"
                                                },
                                                HEARTBEAT_TTL_SECONDS
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
