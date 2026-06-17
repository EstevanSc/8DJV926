use std::collections::HashMap;
use std::sync::{mpsc, Mutex};

use bevy::prelude::*;
use uuid::Uuid;
use common::ability_type::AbilityType;
use game_sockets::GameConnection;

// ---------------------------------------------------------------------------
// Commands sent from the network layer into the simulation
// ---------------------------------------------------------------------------

pub enum SimCommand {
    Joined { connection_id: Uuid, position: Vec2 },
    Left { connection_id: Uuid },
    Input { connection_id: Uuid, dx: f32, dy: f32 },
    GhostJoined { connection_id: Uuid, position: Vec2 },
    GhostPositionUpdate { connection_id: Uuid, position: Vec2 },
    GhostIsNowLocal { connection_id: Uuid, speed: [f64; 2], position: [f64; 2] },
    LocalIsNowGhost { connection_id: Uuid, receiver_shard_id: Uuid },
    CastAbility { entity_id: Uuid, ability_type: AbilityType, direction: Option<Vec2> },
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