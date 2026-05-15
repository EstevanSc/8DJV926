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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://redis:6379".to_string());

    let redis_pool = redis_ops::create_pool(&redis_url)?;

    let state = AppState { redis: redis_pool };

    let app = Router::new()
        .route("/login", post(routes::join::handler))
        .route("/health", get(routes::health::handler))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("Gatekeeper HTTP listening on 0.0.0.0:3000");
    axum::serve(listener, app).await?;

    Ok(())
}
