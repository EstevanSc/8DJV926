mod routes;

use axum::{
    Router,
    routing::{get, post},
};
use common::RedisClient;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Shared application state threaded through Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub redis: RedisClient,
    pub supabase: common::supabase::SupabaseClient,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_string());

    let supabase_url = std::env::var("SUPABASE_URL").expect("SUPABASE_URL env var must be set");
    let supabase_key =
        std::env::var("SUPABASE_SERVICE_KEY").expect("SUPABASE_SERVICE_KEY env var must be set");

    let redis = RedisClient::connect(&redis_url)
        .await
        .map_err(|e| anyhow::anyhow!("Redis connection failed: {e}"))?;
    let supabase = common::supabase::SupabaseClient::new(&supabase_url, supabase_key);

    let state = AppState { redis, supabase };

    let app = Router::new()
        .route("/login", post(routes::join::handler))
        .route("/health", get(routes::health::handler))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("Gatekeeper HTTP listening on 0.0.0.0:3000");
    axum::serve(listener, app).await?;

    Ok(())
}
