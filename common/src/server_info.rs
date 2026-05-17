//! Canonical server metadata shared between Orchestrator and Gatekeeper.

use serde::{Deserialize, Serialize};

/// Full server metadata as stored in Redis and exchanged between services.
///
/// Written by the Orchestrator (heartbeat listener + docker_ops pre-registration),
/// read by the Gatekeeper for player routing decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Unique server identifier (UUID). Matches the Redis key `server:<id>`.
    pub id: String,
    /// Public IP address that clients use to connect.
    pub ip: String,
    /// UDP port the game server is listening on.
    pub port: u16,
    /// Deployment zone (e.g. `"zone_A"`).
    pub zone: String,
    /// Current status: `"starting"`, `"empty"`, `"available"`, or `"full"`.
    pub status: String,
    /// Number of players currently connected.
    pub player_count: usize,
    /// Maximum number of players the server accepts.
    pub max_players: usize,
}
