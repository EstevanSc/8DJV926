use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::AppState;

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
    pub broker_ip: String,
    pub broker_port: u16,
    pub player_name: String,
    pub player_spawn_position: [f32; 2],
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

    let player_spawn_position = match state.supabase.find_player(username).await {
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
            [row.log_out_position_x, row.log_out_position_y]
        }
        Ok(None) => {
            // New player — create the account and log in immediately.
            match state.supabase.create_player(username, password).await {
                Ok(row) => {
                    tracing::info!("New player '{username}' created (id={})", row.id);
                    [row.log_out_position_x, row.log_out_position_y]
                }
                Err(e) => {
                    tracing::error!("Supabase create_player failed: {e:#}");
                    return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "database error"));
                }
            }
        }
    };

    let broker_ip = std::env::var("BROKER_PUBLIC_IP")
        .or_else(|_| std::env::var("BROKER_IP"))
        .unwrap_or_else(|_| "127.0.0.1".into());
    let broker_port = std::env::var("BROKER_PORT")
        .unwrap_or_else(|_| "7776".into())
        .parse()
        .unwrap_or(7776);
    Ok(Json(LoginResponse {
        player_name: username.to_string(),
        player_spawn_position, // Default spawn position
        broker_ip,
        broker_port,
    }))
}
