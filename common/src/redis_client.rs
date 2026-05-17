//! Shared Redis client used by all server-side services.
//!
//! Built on top of `redis::aio::ConnectionManager` which handles
//! reconnection automatically and is safe to clone across tasks.

use redis::aio::ConnectionManager;
use std::collections::HashMap;

/// Async Redis wrapper shared between Orchestrator and Gatekeeper.
///
/// Clone-able — each clone shares the same underlying connection manager.
#[derive(Clone)]
pub struct RedisClient {
    manager: ConnectionManager,
}

impl RedisClient {
    /// Connect to Redis at `redis_url` (e.g. `redis://redis:6379`).
    pub async fn connect(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let manager = ConnectionManager::new(client).await?;
        Ok(RedisClient { manager })
    }

    /// Ping the server — returns `"PONG"` on success.
    pub async fn ping(&self) -> Result<String, redis::RedisError> {
        redis::cmd("PING")
            .query_async(&mut self.manager.clone())
            .await
    }

    /// SET key value.
    pub async fn set(&self, key: &str, value: &str) -> Result<(), redis::RedisError> {
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut self.manager.clone())
            .await
    }

    /// GET key.
    pub async fn get(&self, key: &str) -> Result<String, redis::RedisError> {
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut self.manager.clone())
            .await
    }

    /// HSET key field value [field value …]
    pub async fn hset_multiple(
        &self,
        key: &str,
        fields: HashMap<&str, String>,
    ) -> Result<(), redis::RedisError> {
        let mut cmd = redis::cmd("HSET");
        cmd.arg(key);
        for (field, value) in fields {
            cmd.arg(field).arg(value);
        }
        cmd.query_async(&mut self.manager.clone()).await
    }

    /// EXPIRE key seconds.
    pub async fn expire(&self, key: &str, seconds: usize) -> Result<(), redis::RedisError> {
        redis::cmd("EXPIRE")
            .arg(key)
            .arg(seconds)
            .query_async(&mut self.manager.clone())
            .await
    }

    /// SCAN … MATCH pattern — iterates all matching keys.
    pub async fn scan(&self, pattern: &str) -> Result<Vec<String>, redis::RedisError> {
        let mut keys = Vec::new();
        let mut cursor = 0u64;

        loop {
            let (next_cursor, page): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .query_async(&mut self.manager.clone())
                .await?;

            keys.extend(page);
            cursor = next_cursor;

            if cursor == 0 {
                break;
            }
        }

        Ok(keys)
    }

    /// HGET key field — returns `None` when the key or field is absent.
    pub async fn hget(&self, key: &str, field: &str) -> Result<Option<String>, redis::RedisError> {
        redis::cmd("HGET")
            .arg(key)
            .arg(field)
            .query_async(&mut self.manager.clone())
            .await
    }

    /// DEL key.
    pub async fn del(&self, key: &str) -> Result<(), redis::RedisError> {
        redis::cmd("DEL")
            .arg(key)
            .query_async(&mut self.manager.clone())
            .await
    }

    /// HINCRBY key field increment — atomically increments a hash integer field.
    pub async fn hincr(
        &self,
        key: &str,
        field: &str,
        increment: i64,
    ) -> Result<i64, redis::RedisError> {
        redis::cmd("HINCRBY")
            .arg(key)
            .arg(field)
            .arg(increment)
            .query_async(&mut self.manager.clone())
            .await
    }
}
