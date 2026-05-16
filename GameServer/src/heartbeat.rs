// TEMP, replace with heartbeat in the shared module when merging

use serde::{Deserialize, Serialize};

/// Server status update sent from Dedicated Servers to Orchestrator.
///
/// Used for server discovery and load balancing across zones.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Heartbeat {
    pub id: String,
    pub ip: String,
    pub port: u16,
    pub zone: String,
    pub player_count: usize,
    pub max_players: usize,
}