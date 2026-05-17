//! Server connection information for player routing.
//!
//! Note: ServerInfo is defined here temporarily for the Orchestrator service.
//! When the workspace is unified with Gatekeeper and Dedicated Server,
//! this will be moved to a shared `models/` crate to avoid duplication.

use serde::{Deserialize, Serialize};

/// Shared structure of server address information to be used by Gatekeeper routing decisions.
///
/// Contains minimal data required to direct player connections.
/// Will be shared across services when workspace is unified.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ServerInfo {
    pub ip: String,
    pub port: u16,
    pub zone: String,
}
