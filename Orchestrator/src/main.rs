//! Orchestrator service entry point.
//! Initializes configuration, Redis connection, and HTTP server.

use axum::Router;
use std::net::SocketAddr;
use tokio::signal;

mod api;
mod config;
mod docker_ops;
mod services;

use common::RedisClient;
use config::Config;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    tracing::info!(
        "Starting orchestrator - environment: {}, port: {}, heartbeat_port: {}, redis_url: {}",
        config.environment,
        config.port,
        config.orch_port,
        config.redis_url
    );

    let redis_client = match RedisClient::connect(&config.redis_url).await {
        Ok(client) => {
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

    // Spawn heartbeat listener task if Redis is available
    if let Some(redis) = redis_client {
        let heartbeat_port = config.orch_port;
        let heartbeat_redis = redis.clone();
        let hb_ttl = config.heartbeat_ttl_seconds;
        tokio::spawn(async move {
            tracing::info!("Starting heartbeat listener task");
            services::heartbeat_listener::start_heartbeat_listener(
                heartbeat_port,
                heartbeat_redis,
                hb_ttl,
            )
            .await;
            tracing::error!("Heartbeat listener task stopped unexpectedly");
        });

        let scaler_redis = redis.clone();
        let ds_base_port = config.ds_base_port;
        let hot_servers_min = config.hot_servers_min;
        let scaler_interval = config.scaler_interval_seconds;
        match docker_ops::connect() {
            Ok(docker) => {
                tokio::spawn(async move {
                    tracing::info!("Starting scaler task");
                    services::scaler::start_scaler(
                        docker,
                        scaler_redis,
                        hot_servers_min,
                        ds_base_port,
                        scaler_interval,
                    )
                    .await;
                    tracing::error!("Scaler task stopped unexpectedly");
                });
            }
            Err(e) => {
                tracing::error!("Failed to connect to Docker daemon — scaler disabled: {e}");
            }
        }
    } else {
        tracing::warn!("Background tasks not started: Redis connection required");
    }
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
