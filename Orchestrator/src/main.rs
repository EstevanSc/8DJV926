use axum::Router;
use std::net::SocketAddr;
use tokio::signal;

mod api;
mod config;
mod infrastructure;

use config::Config;
use infrastructure::RedisClient;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    tracing::info!(
        "Starting orchestrator - environment: {}, port: {}, redis_url: {}",
        config.environment,
        config.port,
        config.redis_url
    );

    #[allow(unused_mut, unused_variables)]
    let mut redis_client = match RedisClient::connect(&config.redis_url).await {
        Ok(mut client) => {
            tracing::info!("Successfully connected to Redis");
            match client.ping().await {
                Ok(pong) => {
                    tracing::info!("Redis ping successful: {}", pong);
                }
                Err(e) => {
                    tracing::error!("Redis ping failed: {}", e);
                }
            }

            if let Err(e) = client.set("orchestrator:status", "online").await {
                tracing::error!("Failed to set orchestrator:status: {}", e);
            } else {
                match client.get("orchestrator:status").await {
                    Ok(value) => {
                        tracing::info!("Orchestrator status: {}", value);
                    }
                    Err(e) => {
                        tracing::error!("Failed to read orchestrator:status: {}", e);
                    }
                }
            }

            Some(client)
        }
        Err(e) => {
            tracing::error!("Failed to connect to Redis: {}", e);
            None
        }
    };

    let app = Router::new().nest("/api", api::routes());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind to address");

    tracing::info!("Server listening on {}", addr);

    let shutdown = async {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("Failed to install CTRL+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
    };

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .expect("Server error");

    tracing::info!("Orchestrator shutting down");
}
