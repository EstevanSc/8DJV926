use redis::aio::ConnectionManager;

pub struct RedisClient {
    manager: ConnectionManager,
}

impl RedisClient {
    pub async fn connect(redis_url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(redis_url)?;
        let manager = ConnectionManager::new(client).await?;
        Ok(RedisClient { manager })
    }

    pub async fn ping(&mut self) -> Result<String, redis::RedisError> {
        redis::cmd("PING").query_async(&mut self.manager).await
    }
}
