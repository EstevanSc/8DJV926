mod db;
mod redis_ops;
mod routes;

use axum::{
    Router,
    routing::{get, post},
};
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Shared application state threaded through Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub redis: deadpool_redis::Pool,
    pub supabase: db::SupabaseClient,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://redis:6379".to_string());

    let supabase_url = std::env::var("SUPABASE_URL")
        .expect("SUPABASE_URL env var must be set");
    let supabase_key = std::env::var("SUPABASE_SERVICE_KEY")
        .expect("SUPABASE_SERVICE_KEY env var must be set");

    let redis_pool = redis_ops::create_pool(&redis_url)?;
    let supabase = db::SupabaseClient::new(&supabase_url, supabase_key);

    let state = AppState { redis: redis_pool, supabase };

    let app = Router::new()
        .route("/login", post(routes::join::handler))
        .route("/health", get(routes::health::handler))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("Gatekeeper HTTP listening on 0.0.0.0:3000");
    axum::serve(listener, app).await?;

    Ok(())
}
