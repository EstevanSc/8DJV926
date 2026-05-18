use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};

// ---------------------------------------------------------------------------
// Unreliable datagrams — sent every tick via QUIC unreliable datagrams.
// ---------------------------------------------------------------------------

/// Position and velocity snapshot for a single entity.
/// Sent unreliably every tick; dropped packets are simply skipped.
/// Entity IDs are u32 — never UUIDs — to keep datagrams small.
#[derive(Debug, Clone, Copy, Encode, Decode, Serialize, Deserialize, SchemaWrite, SchemaRead)]
pub struct PositionSnapshot {
    pub entity_id: u32,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
}

/// A batch of position snapshots sent to one client per tick.
#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize, SchemaWrite, SchemaRead)]
pub struct PositionBatch {
    pub tick: u32,
    pub snapshots: Vec<PositionSnapshot>,
}

// ---------------------------------------------------------------------------
// Auth handshake packets (client ↔ gatekeeper)
// ---------------------------------------------------------------------------

/// Sent by the client to the gatekeeper over QUIC to establish a game session.
/// `player_id` is the UUID received from the HTTP `/login` response.
#[derive(Debug, Clone, Encode, Decode)]
pub struct ConnectRequest {
    pub player_id: String,
}

/// Sent by the gatekeeper to the client after a successful connection.
/// Contains the entity ID assigned to this player for this session.
#[derive(Debug, Clone, Copy, Encode, Decode)]
pub struct AuthAck {
    pub entity_id: u32,
}

/// Player movement input — sent as unreliable datagram from client to server each frame.
/// `dx`/`dy` are in the range [-1, 1]; the server normalises and scales by speed.
#[derive(Debug, Clone, Copy, Encode, Decode, Serialize, Deserialize, SchemaWrite, SchemaRead)]
pub struct PlayerInput {
    pub dx: f32,
    pub dy: f32,
}
