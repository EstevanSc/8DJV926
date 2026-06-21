pub struct Config {
    pub broker_host: String,
    pub broker_port: u16,
    pub supabase_url: String,
    pub supabase_key: String,
}

impl Config {
    pub fn from_env() -> Self {
        dotenv::dotenv().ok();

        let broker_host = std::env::var("BROKER_HOST").unwrap_or_else(|_| "broker".to_string());

        let broker_port = std::env::var("BROKER_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7776);

        let supabase_url = std::env::var("SUPABASE_URL").expect("SUPABASE_URL env var must be set");

        let supabase_key = std::env::var("SUPABASE_SERVICE_KEY")
            .expect("SUPABASE_SERVICE_KEY env var must be set");

        Config {
            broker_host,
            broker_port,
            supabase_url: supabase_url.trim().to_string(),
            supabase_key: supabase_key.trim().to_string(),
        }
    }
}
