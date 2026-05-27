use std::env;

use bevy::prelude::*;

use super::messages::HandoffRequest;

/// Local authority state for a simulation entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component, Default)]
pub enum AuthorityState {
    #[default]
    Owned,
    PendingHandoff,
    Ghost,
}

impl AuthorityState {
    /// Returns true when the entity can still be simulated locally.
    pub fn allows_local_simulation(self) -> bool {
        matches!(self, Self::Owned | Self::PendingHandoff)
    }

    /// Returns true when the entity should be hidden from client snapshots.
    pub fn is_snapshot_visible(self) -> bool {
        !matches!(self, Self::Ghost)
    }

    /// Returns true when the entity is a ghost replica.
    pub fn is_ghost(self) -> bool {
        matches!(self, Self::Ghost)
    }
}

/// Read-only replica metadata for a remote entity.
#[derive(Debug, Clone, Copy, Component)]
pub struct GhostReplica {
    pub source_shard_id: u32,
    pub source_entity_id: u32,
}

/// Pending handoff data for a local entity.
#[derive(Debug, Clone, Component)]
pub struct HandoffRequestState {
    pub target_shard_id: u32,
    pub request: HandoffRequest,
    pub requested_tick: u32,
    pub dispatched: bool,
}

/// Runtime configuration for authority behavior.
#[derive(Resource, Debug, Clone)]
pub struct AuthorityConfig {
    pub local_shard_id: u32,
    pub handoff_margin: f32,
}

impl Default for AuthorityConfig {
    /// Builds config from environment variables.
    fn default() -> Self {
        let local_shard_id = env::var("DS_SHARD_ID")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);

        Self {
            local_shard_id,
            handoff_margin: env::var("DS_HANDOFF_MARGIN")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(48.0),
        }
    }
}