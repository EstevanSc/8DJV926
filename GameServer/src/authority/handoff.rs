use bevy::prelude::*;
use uuid::Uuid;
use tracing::{debug, info};

use super::components::{AuthorityState, GhostReplica, HandoffRequestState};
use super::messages::{AuthorityEnvelope, HandoffRequest};
use crate::simulation::Player;

/// Builds a handoff request payload.
pub fn build_handoff_request(
    entity_id: u32,
    pos: Vec2,
    vel: Vec2,
    state: [u8; 64],
) -> HandoffRequest {
    HandoffRequest {
        entity_id,
        pos,
        vel,
        state,
    }
}

/// Marks an entity as pending handoff.
pub fn begin_handoff(
    commands: &mut Commands,
    entity: Entity,
    target_shard_uuid: Uuid,
    request: HandoffRequest,
    tick: u32,
) {
    info!(
        entity_id = request.entity_id,
        target_shard_uuid = ?target_shard_uuid,
        tick,
        "Marking entity as pending handoff"
    );
    commands.entity(entity).insert((
        AuthorityState::PendingHandoff,
        HandoffRequestState {
            target_shard_uuid,
            request,
            requested_tick: tick,
            dispatched: false,
            accepted: false,
        },
    ));
}

/// Marks a pending handoff as accepted.
pub fn accept_handoff(request_state: &mut HandoffRequestState) {
    request_state.accepted = true;
}

/// Restores local authority after a handoff success.
pub fn finalize_handoff(commands: &mut Commands, entity: Entity) {
    debug!(?entity, "Restoring local authority after handoff success");
    commands.entity(entity).insert(AuthorityState::Owned);
    commands.entity(entity).remove::<HandoffRequestState>();
    commands.entity(entity).remove::<GhostReplica>();
}

/// Restores local authority after a handoff rejection.
pub fn reject_handoff(commands: &mut Commands, entity: Entity) {
    debug!(?entity, "Restoring local authority after handoff rejection");
    commands.entity(entity).insert(AuthorityState::Owned);
    commands.entity(entity).remove::<HandoffRequestState>();
    commands.entity(entity).remove::<GhostReplica>();
}

/// Downgrades local authority to a ghost after a handoff success.
pub fn downgrade_to_ghost(commands: &mut Commands, entity: Entity) {
    debug!(?entity, "Downgrading to ghost after handoff success");
    commands.entity(entity).insert(AuthorityState::Ghost);
    commands.entity(entity).remove::<HandoffRequestState>();
}

/// Wraps the current entity state into an outbound handoff request.
pub fn encode_request(player: &Player, transform: &Transform, velocity: Vec2) -> AuthorityEnvelope {
    let request = HandoffRequest {
        entity_id: player.entity_id,
        pos: transform.translation.truncate(),
        vel: velocity,
        state: [0u8; 64],
    };

    AuthorityEnvelope::HandoffRequest(request)
}
