use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{redis_ops, AppState};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct ServerInfo {
    pub ip: String,
    pub port: u16,
    pub zone: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub player_id: String,
    pub server: ServerInfo,
}

pub async fn handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    // Validate username at the system boundary.
    // Password is always accepted — real auth will be implemented later.
    let username = body.username.trim();
    if username.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut conn = state.redis.get().await.map_err(|e| {
        tracing::error!("Redis pool error during login: {e}");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Find an available server from Redis.
    let server = redis_ops::find_available_server(&mut conn)
        .await
        .map_err(|e| {
            tracing::error!("find_available_server failed: {e:#}");
            StatusCode::SERVICE_UNAVAILABLE
        })?;

    let Some(server) = server else {
        tracing::warn!("No available server for player '{username}'");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };

    // Record the new player on the chosen server.
    redis_ops::increment_player_count(&mut conn, &server.id)
        .await
        .map_err(|e| {
            tracing::error!("increment_player_count failed: {e:#}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let player_id = Uuid::new_v4().to_string();

    tracing::info!(
        %player_id,
        server_id = %server.id,
        "Player '{username}' assigned → {}:{} ({})",
        server.ip,
        server.port,
        server.zone,
    );

    Ok(Json(LoginResponse {
        player_id,
        server: ServerInfo {
            ip: server.ip,
            port: server.port,
            zone: server.zone,
        },
    }))
}
