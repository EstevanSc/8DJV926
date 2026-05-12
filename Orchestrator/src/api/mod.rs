pub mod health;

use axum::{routing::get, Router};

pub fn routes() -> Router {
    Router::new().route("/health", get(health::health_check))
}
