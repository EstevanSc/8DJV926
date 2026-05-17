use anyhow::Context;
use deadpool_redis::{Config, Pool, Runtime};
use deadpool_redis::redis::AsyncCommands;
use common::redis_keys;

pub fn create_pool(url: &str) -> anyhow::Result<Pool> {
    Config::from_url(url)
        .create_pool(Some(Runtime::Tokio1))
        .context("failed to create Redis pool")
}

// ---------------------------------------------------------------------------
// Server discovery
// ---------------------------------------------------------------------------

pub struct ServerInfo {
    pub id: String,
    pub ip: String,
    pub port: u16,
    pub zone: String,
}

/// Return the first available server (status == "available") from Redis, or
/// `None` if no server is ready to accept players.
pub async fn find_available_server(
    conn: &mut deadpool_redis::Connection,
) -> anyhow::Result<Option<ServerInfo>> {
    let server_ids: Vec<String> = conn
        .smembers(redis_keys::active_servers_key())
        .await
        .context("SMEMBERS servers:active")?;

    for id in server_ids {
        let fields: std::collections::HashMap<String, String> = conn
            .hgetall(redis_keys::server_key(&id))
            .await
            .context("HGETALL server")?;

        let status = fields.get("status").map(String::as_str).unwrap_or("");
        if status != "available" {
            continue;
        }

        let ip = fields.get("ip").cloned().unwrap_or_default();
        let port: u16 = fields.get("port").and_then(|v| v.parse().ok()).unwrap_or(0);
        let zone = fields.get("zone").cloned().unwrap_or_default();

        return Ok(Some(ServerInfo { id, ip, port, zone }));
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Player count tracking
// ---------------------------------------------------------------------------

/// Increment the `players` counter on a server hash when a login succeeds.
pub async fn increment_player_count(
    conn: &mut deadpool_redis::Connection,
    server_id: &str,
) -> anyhow::Result<()> {
    let (): () = conn
        .hincr(redis_keys::server_key(server_id), "players", 1_i64)
        .await
        .context("HINCRBY server players")?;
    Ok(())
}
