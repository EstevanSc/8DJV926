use axum::{Json, extract::State, http::StatusCode};
use common::ServerInfo;
use serde::{Deserialize, Serialize};

use crate::{AppState, redis_ops};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub player_id: String,
    pub server: ServerInfo,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (status, Json(ErrorResponse { error: msg.into() }))
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let username = body.username.trim();
    let password = body.password.trim();

    if username.is_empty() || password.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "username and password cannot be empty",
        ));
    }

    // ── Supabase: find or create the player ─────────────────────────────────

    let player_id = match state.supabase.find_player(username).await {
        Err(e) => {
            tracing::error!("Supabase find_player failed: {e:#}");
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "database error"));
        }
        Ok(Some(row)) => {
            // Player exists — verify password.
            if row.password != password {
                tracing::warn!("Failed login attempt for '{username}': wrong password");
                return Err(err(StatusCode::UNAUTHORIZED, "invalid password"));
            }
            tracing::info!("Existing player '{username}' authenticated (id={})", row.id);
            row.id.to_string()
        }
        Ok(None) => {
            // New player — create the account and log in immediately.
            match state.supabase.create_player(username, password).await {
                Ok(row) => {
                    tracing::info!("New player '{username}' created (id={})", row.id);
                    row.id.to_string()
                }
                Err(e) => {
                    tracing::error!("Supabase create_player failed: {e:#}");
                    return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "database error"));
                }
            }
        }
    };

    // ── Redis: pick an available game server ────────────────────────────────

    let server = redis_ops::find_available_server(&state.redis)
        .await
        .map_err(|e| {
            tracing::error!("find_available_server failed: {e:#}");
            err(StatusCode::SERVICE_UNAVAILABLE, "no server available")
        })?;

    let Some(server) = server else {
        tracing::warn!("No available server for player '{username}'");
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "no server available"));
    };

    redis_ops::increment_player_count(&state.redis, &server.id)
        .await
        .map_err(|e| {
            tracing::error!("increment_player_count failed: {e:#}");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server assignment failed",
            )
        })?;

    tracing::info!(
        player_id = %player_id,
        server_id = %server.id,
        "Player '{username}' assigned → {}:{} ({})",
        server.ip,
        server.port,
        server.zone,
    );

    Ok(Json(LoginResponse { player_id, server }))
}
