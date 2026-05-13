//! Redis client abstraction for async operations.
//! Provides connection management and basic SET/GET/PING commands.

use redis::aio::ConnectionManager;

pub struct RedisClient {
    manager: ConnectionManager,
}

impl RedisClient {
    /// Connects to Redis using the provided URL and returns a RedisClient instance.
    pub async fn connect(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let manager = ConnectionManager::new(client).await?;
        Ok(RedisClient { manager })
    }

    /// Pings the Redis server to check connectivity. Returns "PONG" if successful.
    pub async fn ping(&mut self) -> Result<String, redis::RedisError> {
        redis::cmd("PING").query_async(&mut self.manager).await
    }

    /// Sets a key-value pair in Redis. Returns Ok(()) if successful.
    pub async fn set(&mut self, key: &str, value: &str) -> Result<(), redis::RedisError> {
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut self.manager)
            .await
    }

    /// Gets the value of a key from Redis. Returns the value as a String if successful.
    pub async fn get(&mut self, key: &str) -> Result<String, redis::RedisError> {
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut self.manager)
            .await
    }
}
