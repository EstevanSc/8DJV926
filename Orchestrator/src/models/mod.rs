//! Shared server metadata structures for MMO infrastructure.
//!
//! Defines data structures used across services:
//! - Orchestrator: heartbeat receiver and server registry
//! - Dedicated Servers: heartbeat sender
//! - Gatekeeper: server selection for player connections

pub mod heartbeat;
pub mod server_info;

#[allow(unused_imports)]
pub use heartbeat::Heartbeat;
#[allow(unused_imports)]
pub use server_info::ServerInfo;
