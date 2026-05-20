//! Redis helpers for Gatekeeper — server discovery and player-count tracking.
//!
//! Uses `common::RedisClient` (backed by `redis::ConnectionManager`) to query
//! the server registry maintained by the Orchestrator. Server entries live at
//! `server:<uuid>` and are written / refreshed by the Orchestrator's heartbeat
//! listener with a TTL, so stale entries disappear automatically.

use anyhow::Context;
use common::{RedisClient, ServerInfo, redis_keys};

// ---------------------------------------------------------------------------
// Server discovery
// ---------------------------------------------------------------------------

/// Return the best server with a free slot from Redis, or `None` if no server
/// is ready.
///
/// Selection priority:
/// 1. `"available"` servers (1+ players, not yet full) — sorted by
///    `player_count` descending so existing sessions are filled first.
/// 2. `"empty"` servers (0 players, hot-standby) — only used when no
///    `"available"` server exists.
///
/// Uses `SCAN server:*` to match entries written by the Orchestrator, which is
/// consistent with how the scaler enumerates the fleet.
pub async fn find_available_server(redis: &RedisClient) -> anyhow::Result<Option<ServerInfo>> {
    let keys = redis.scan("server:*").await.context("SCAN server:*")?;

    let mut candidates: Vec<ServerInfo> = Vec::new();

    for key in keys {
        let status = match redis.hget(&key, "status").await.context("HGET status")? {
            Some(s) => s,
            None => continue,
        };

        // "empty"     = 0 players (hot-standby server)
        // "available" = some players but not yet full
        if status != "empty" && status != "available" {
            continue;
        }

        let id = key.strip_prefix("server:").unwrap_or(&key).to_string();
        let ip = redis.hget(&key, "ip").await?.unwrap_or_default();
        let port: u16 = redis
            .hget(&key, "port")
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let zone = redis.hget(&key, "zone").await?.unwrap_or_default();
        let player_count: usize = redis
            .hget(&key, "player_count")
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let max_players: usize = redis
            .hget(&key, "max_players")
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        candidates.push(ServerInfo {
            id,
            ip,
            port,
            zone,
            status,
            player_count,
            max_players,
        });
    }

    // Sort: "available" (status != "empty") before "empty", then by
    // player_count descending so the fullest available server is filled first.
    candidates.sort_by(|a, b| {
        let a_empty = a.status == "empty";
        let b_empty = b.status == "empty";
        match (a_empty, b_empty) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => b.player_count.cmp(&a.player_count),
        }
    });

    Ok(candidates.into_iter().next())
}

// ---------------------------------------------------------------------------
// Player-count tracking
// ---------------------------------------------------------------------------

/// Atomically increment the `player_count` field on a server hash.
///
/// This bridges the gap between heartbeats — the actual count is overwritten
/// by the next heartbeat from the game server, so the increment is a temporary
/// approximation that prevents double-routing.
pub async fn increment_player_count(redis: &RedisClient, server_id: &str) -> anyhow::Result<()> {
    redis
        .hincr(&redis_keys::server_key(server_id), "player_count", 1)
        .await
        .context("HINCRBY server player_count")?;
    Ok(())
}
