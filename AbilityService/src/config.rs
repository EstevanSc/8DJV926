
pub struct Config {
    pub broker_host: String,
    pub broker_port: u16,
    pub ability_service_tick_ms: u64,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();

        Config {
            broker_host: std::env::var("QUADTREE_BROKER_HOST")
                .unwrap_or_else(|_| "broker".to_string()),
            broker_port: std::env::var("QUADTREE_BROKER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(7776),
            ability_service_tick_ms: std::env::var("ABILITY_SERVICE_TICK_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
        }
    }
}