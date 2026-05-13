//! Server connection information for player routing.

use serde::{Deserialize, Serialize};

/// Server address information for Gatekeeper routing decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ServerInfo {
    pub ip: String,
    pub port: u16,
    pub zone: String,
}
