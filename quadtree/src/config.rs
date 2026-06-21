pub struct Config {
    pub world_size: f64,
    pub max_capacity: usize,
    pub max_depth: u8,
    pub nearby_margin: f64,
    pub orchestrator_host: String,
    pub orchestrator_port: u16,
    pub broker_host: String,
    pub broker_port: u16,
    pub quadtree_tick_ms: u64,
    pub area_of_interest_radius: f64,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();

        Config {
            world_size: std::env::var("QUADTREE_WORLD_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
            max_capacity: std::env::var("QUADTREE_MAX_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            max_depth: std::env::var("QUADTREE_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            nearby_margin: std::env::var("QUADTREE_NEARBY_MARGIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5.0),
            orchestrator_host: std::env::var("QUADTREE_ORCHESTRATOR_HOST")
                .unwrap_or_else(|_| "localhost".to_string()),
            orchestrator_port: std::env::var("QUADTREE_ORCHESTRATOR_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            broker_host: std::env::var("QUADTREE_BROKER_HOST")
                .unwrap_or_else(|_| "broker".to_string()),
            broker_port: std::env::var("QUADTREE_BROKER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(7776),
            quadtree_tick_ms: std::env::var("QUADTREE_TICK_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            area_of_interest_radius: std::env::var("QUADTREE_AREA_OF_INTEREST_RADIUS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
        }
    }
}
