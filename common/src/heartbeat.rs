//! Heartbeat payload sent by game servers to the Orchestrator over UDP.

use serde::{Deserialize, Serialize};

/// Status ping sent from a game server every few seconds.
///
/// The Orchestrator persists this into Redis (`server:<id>`) and refreshes
/// the TTL so stale entries expire automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Unique server ID — matches `DS_ID` env var injected by docker_ops.
    pub id: String,
    /// Public IP address clients use to connect.
    pub ip: String,
    /// UDP port the server is listening on.
    pub port: u16,
    /// Zone identifier.
    pub zone: String,
    /// Current connected player count.
    pub player_count: usize,
    /// Maximum player capacity.
    pub max_players: usize,
}
