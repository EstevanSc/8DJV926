use std::collections::{HashMap, VecDeque};

use avian2d::prelude::LinearVelocity;
use bevy::prelude::*;
use tracing::{debug, info, trace, warn};

use super::components::{AuthorityState, GhostReplica, HandoffRequestState};
use super::ghost::{apply_ghost_update, spawn_ghost_entity};
use super::handoff::{accept_handoff, downgrade_to_ghost, finalize_handoff, reject_handoff};
use super::messages::{
    AuthorityEnvelope, GhostUpdate, HandoffAccept, HandoffComplete, HandoffReject, HandoffRequest,
};
use crate::simulation::{Player, TickCounter};

/// Local inbox and outbox for authority traffic.
#[derive(Resource, Default)]
pub struct AuthorityBus {
    pub inbound: VecDeque<AuthorityEnvelope>, 
    pub outbound: VecDeque<AuthorityEnvelope>, // No outbound for now
    pub pending_requests: VecDeque<HandoffRequest>,
    pub pending_accepts: VecDeque<HandoffAccept>,
    pub pending_rejects: VecDeque<HandoffReject>,
    pub pending_ghost_updates: VecDeque<GhostUpdate>,
    pub pending_completes: VecDeque<HandoffComplete>,
}

type HandoffRequestQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Player,
        &'static Transform,
        Option<&'static mut HandoffRequestState>,
        &'static AuthorityState,
        Option<&'static LinearVelocity>,
    ),
>;

type GhostUpdateQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Transform,
        &'static AuthorityState,
        &'static GhostReplica,
    ),
>;

type HandoffCompleteQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Player,
        &'static AuthorityState,
        Option<&'static mut HandoffRequestState>,
    ),
>;

/// Routes raw inbound messages into typed queues.
pub fn route_inbound_messages(mut bus: ResMut<AuthorityBus>) {
    while let Some(message) = bus.inbound.pop_front() {
        trace!(message = ?message, "Authority inbound message received");
        match message {
            AuthorityEnvelope::HandoffRequest(request) => {
                debug!(entity_id = request.entity_id, "Queued handoff request");
                bus.pending_requests.push_back(request);
            }
            AuthorityEnvelope::HandoffAccept(accept) => {
                debug!(entity_id = accept.entity_id, "Queued handoff accept");
                bus.pending_accepts.push_back(accept);
            }
            AuthorityEnvelope::HandoffReject(reject) => {
                warn!(entity_id = reject.entity_id, "Queued handoff reject");
                bus.pending_rejects.push_back(reject);
            }
            AuthorityEnvelope::GhostUpdate(update) => {
                trace!(entity_id = update.entity_id, "Queued ghost update");
                bus.pending_ghost_updates.push_back(update); // Queue ghost update for processing in authority/systems.rs/apply_ghost_updates
            }
            AuthorityEnvelope::HandoffComplete(complete) => {
                info!(entity_id = complete.entity_id, "Queued handoff complete");
                bus.pending_completes.push_back(complete);
            }
        }
    }
}

/// Emits outbound handoff requests for pending entities.
pub fn apply_handoff_requests(
    mut bus: ResMut<AuthorityBus>,
    mut query: HandoffRequestQuery<'_, '_>,
    tick: Res<TickCounter>,
) {
    for (player, transform, request_state, authority_state, velocity) in &mut query {
        let Some(mut request_state) = request_state else {
            continue;
        };

        if request_state.dispatched || !matches!(*authority_state, AuthorityState::PendingHandoff) {
            continue;
        }

        let vel = velocity
            .map(|value| Vec2::new(value.x, value.y))
            .unwrap_or(Vec2::ZERO);

        let mut request = request_state.request;
        request.entity_id = player.entity_id;
        request.pos = transform.translation.truncate();
        request.vel = vel;

        info!(
            entity_id = player.entity_id,
            target_shard_uuid = ?request_state.target_shard_uuid,
            tick = tick.0,
            "Dispatching handoff request"
        );
        bus.outbound
            .push_back(AuthorityEnvelope::HandoffRequest(request));
        request_state.requested_tick = tick.0;
        request_state.dispatched = true;
    }
}

/// Applies queued ghost updates to local replicas.
pub fn apply_ghost_updates(
    mut commands: Commands,
    mut bus: ResMut<AuthorityBus>,
    mut query: GhostUpdateQuery<'_, '_>,
) {
    let mut latest_updates: HashMap<u32, Vec2> = HashMap::new();

    while let Some(update) = bus.pending_ghost_updates.pop_front() {
        latest_updates.insert(update.entity_id, update.pos);
    }

    for (entity_id, position) in latest_updates {
        let mut matching_ghosts = Vec::new();

        for (entity, mut transform, state, ghost) in &mut query {
            if matches!(*state, AuthorityState::Ghost) && ghost.source_entity_id == entity_id {
                matching_ghosts.push((entity, ghost.source_shard_id));
                trace!(
                    entity_id,
                    source_shard_id = ghost.source_shard_id,
                    "Applying ghost update"
                );
                apply_ghost_update(&mut transform, position);
            }
        }

        if matching_ghosts.is_empty() {
            debug!(entity_id, "Spawning ghost entity from first ghost update");
            spawn_ghost_entity(&mut commands, entity_id, 0, position);
            continue;
        }

        for (duplicate_entity, source_shard_id) in matching_ghosts.into_iter().skip(1) {
            warn!(
                entity_id,
                source_shard_id,
                duplicate_entity = ?duplicate_entity,
                "Despawning duplicate ghost entity"
            );
            commands.entity(duplicate_entity).despawn();
        }
    }
}

/// Completes or rejects pending handoffs.
pub fn complete_handoffs(
    mut bus: ResMut<AuthorityBus>,
    mut commands: Commands,
    mut query: HandoffCompleteQuery<'_, '_>,
) {
    while let Some(accept) = bus.pending_accepts.pop_front() {
        for (_entity, player, state, request_state) in &mut query {
            if player.entity_id == accept.entity_id
                && matches!(*state, AuthorityState::PendingHandoff)
            {
                info!(entity_id = accept.entity_id, "Handoff accepted");
                if let Some(mut request_state) = request_state {
                    accept_handoff(&mut request_state);
                }
                break;
            }
        }
    }

    while let Some(reject) = bus.pending_rejects.pop_front() {
        for (entity, player, state, _) in &mut query {
            if player.entity_id == reject.entity_id
                && matches!(*state, AuthorityState::PendingHandoff)
            {
                warn!(entity_id = reject.entity_id, "Handoff rejected");
                reject_handoff(&mut commands, entity);
                break;
            }
        }
    }

    let mut remaining_completes = VecDeque::new();

    while let Some(complete) = bus.pending_completes.pop_front() {
        let mut finalized = false;

        for (entity, player, state, request_state) in &mut query {
            if player.entity_id == complete.entity_id
                && matches!(*state, AuthorityState::PendingHandoff)
            {
                if request_state.as_ref().is_some_and(|request_state| request_state.accepted) {
                    info!(entity_id = complete.entity_id, "Handoff completed - Relinquishing authority");
                    downgrade_to_ghost(&mut commands, entity);
                    finalized = true;
                }
                break;
            }
        }

        if !finalized {
            remaining_completes.push_back(complete);
        }
    }

    bus.pending_completes = remaining_completes;
}
