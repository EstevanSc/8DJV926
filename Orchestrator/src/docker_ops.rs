//! Docker container lifecycle management for game-server instances.
//!
//! The orchestrator uses the Docker API (via bollard) to spawn and stop
//! dedicated game-server containers instead of spawning OS processes.
//! The Docker socket must be mounted into the orchestrator container:
//!   volumes:
//!     - /var/run/docker.sock:/var/run/docker.sock

use std::collections::HashMap;

use anyhow::Context;
use bollard::Docker;
use bollard::container::{Config, CreateContainerOptions, NetworkingConfig, StartContainerOptions};
use bollard::models::{EndpointSettings, HostConfig, PortBinding};
use uuid::Uuid;

use crate::infrastructure::RedisClient;

// ── env-var helpers ──────────────────────────────────────────────────────────

fn game_image() -> String {
    std::env::var("GAME_IMAGE").unwrap_or_else(|_| "game-server:latest".to_string())
}

fn game_network() -> String {
    std::env::var("GAME_NETWORK").unwrap_or_else(|_| "game-net".to_string())
}

// ── public API ───────────────────────────────────────────────────────────────

/// Connect to the Docker daemon via the platform-default socket.
pub fn connect() -> anyhow::Result<Docker> {
    Docker::connect_with_socket_defaults().context("connect to Docker socket")
}

/// Stop and remove a container by its Docker container ID.
pub async fn stop_container(docker: &Docker, container_id: &str) -> anyhow::Result<()> {
    docker
        .stop_container(container_id, None)
        .await
        .context("stop container")?;
    docker
        .remove_container(container_id, None)
        .await
        .context("remove container")?;
    Ok(())
}

/// Spawn a new game-server container on the given UDP port.
///
/// - Creates and starts the container on the configured Docker network.
/// - Passes `DS_ID=<server_id>` so the game server uses the same ID as the
///   Redis key, ensuring heartbeats update the correct entry.
/// - Pre-registers `server:<id>` in Redis with `status=starting`; the
///   heartbeat listener promotes it to `empty` once the server is running.
pub async fn spawn_server(
    docker: &Docker,
    redis: &RedisClient,
    port: u16,
) -> anyhow::Result<String> {
    let server_id = Uuid::new_v4().to_string();

    // PUBLIC_ADDR is the address clients use to reach this server.
    // Defaults to "localhost" for local dev with Docker port-mapping.
    // Set the orchestrator env var PUBLIC_ADDR to override in production.
    let public_addr = std::env::var("PUBLIC_ADDR").unwrap_or_else(|_| "localhost".to_string());
    let zone = std::env::var("DS_ZONE").unwrap_or_else(|_| "zone_A".to_string());
    let max_players = std::env::var("MAX_PLAYERS").unwrap_or_else(|_| "2".to_string());
    let orch_host = std::env::var("ORCH_HOST").unwrap_or_else(|_| "orchestrator:7000".to_string());
    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

    let env = vec![
        format!("DS_ID={server_id}"),
        format!("DS_PORT={port}"),
        format!("DS_ZONE={zone}"),
        format!("MAX_PLAYERS={max_players}"),
        format!("ORCH_HOST={orch_host}"),
        format!("RUST_LOG={rust_log}"),
    ];

    // Bind the QUIC UDP port to the host so clients can reach the container.
    let port_key = format!("{port}/udp");
    let host_config = HostConfig {
        port_bindings: Some(HashMap::from([(
            port_key,
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(port.to_string()),
            }]),
        )])),
        ..Default::default()
    };

    let network_name = game_network();
    let networking_config = NetworkingConfig {
        endpoints_config: HashMap::from([(
            network_name,
            EndpointSettings {
                ..Default::default()
            },
        )]),
    };

    let container_name = format!("game-{server_id}");
    let options = CreateContainerOptions {
        name: container_name,
        platform: None,
    };

    let config: Config<String> = Config {
        image: Some(game_image()),
        env: Some(env),
        host_config: Some(host_config),
        networking_config: Some(networking_config),
        labels: Some(HashMap::from([
            ("mmo.role".to_string(), "game-server".to_string()),
            ("mmo.server-id".to_string(), server_id.clone()),
        ])),
        ..Default::default()
    };

    let create_resp = docker
        .create_container(Some(options), config)
        .await
        .context("create game-server container")?;

    let container_id = create_resp.id;

    docker
        .start_container(&container_id, None::<StartContainerOptions<String>>)
        .await
        .context("start game-server container")?;

    // Pre-register in Redis. The heartbeat listener will overwrite these fields
    // (except container_id) once the server starts sending heartbeats, moving
    // status from "starting" → "empty".
    let redis_key = format!("server:{server_id}");
    let mut fields: HashMap<&str, String> = HashMap::new();
    fields.insert("container_id", container_id.clone());
    fields.insert("ip", public_addr);
    fields.insert("port", port.to_string());
    fields.insert("zone", zone);
    fields.insert("status", "starting".to_string());
    fields.insert("players", "0".to_string());
    redis
        .hset_multiple(&redis_key, fields)
        .await
        .context("pre-register server in Redis")?;

    tracing::info!(
        server_id = %server_id,
        container_id = %container_id,
        port,
        "Game server container started",
    );

    Ok(server_id)
}
