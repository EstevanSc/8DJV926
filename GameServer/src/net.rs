use std::collections::HashMap;
use std::sync::{mpsc, Mutex};

use bevy::prelude::*;
use uuid::Uuid;

use game_sockets::GameConnection;

// ---------------------------------------------------------------------------
// Commands sent from the network layer into the simulation
// ---------------------------------------------------------------------------

pub enum SimCommand {
    Joined { entity_id: u32, display_name: String },
    Left { entity_id: u32 },
    Input { entity_id: u32, dx: f32, dy: f32 },
    
    CrossingAlert { entity_id: u32, target_shard_id: u32 },
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Sending half of the sim-command channel. Held by server systems.
#[derive(Resource, Clone)]
pub struct SimCommandSender(pub mpsc::Sender<SimCommand>);

/// Receiving half of the sim-command channel. Drained every tick by the simulation.
#[derive(Resource)]
pub struct SimCommandReceiver(pub Mutex<mpsc::Receiver<SimCommand>>);

/// All currently connected players, keyed by connection UUID.
/// Used by the simulation to broadcast position snapshots each tick.
#[derive(Resource, Default)]
pub struct ConnectedPlayers(pub Mutex<HashMap<Uuid, GameConnection>>);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive a stable `u32` entity ID from a connection UUID.
/// Must match the derivation performed on the client side.
pub fn entity_id_from_uuid(id: Uuid) -> u32 {
    id.as_bytes()
        .iter()
        .fold(0u32, |acc, &b| acc.wrapping_add(b as u32))
}
