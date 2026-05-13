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

    pub async fn set(&mut self, key: &str, value: &str) -> Result<(), redis::RedisError> {
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut self.manager)
            .await
    }

    pub async fn get(&mut self, key: &str) -> Result<String, redis::RedisError> {
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut self.manager)
            .await
    }
}
