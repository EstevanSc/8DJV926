pub mod health;

use axum::{Router, routing::get};

pub fn routes() -> Router {
    Router::new().route("/health", get(health::health_check))
}
