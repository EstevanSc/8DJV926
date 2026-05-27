use std::collections::VecDeque;

use avian2d::prelude::LinearVelocity;
use bevy::prelude::*;
use tracing::{debug, info, trace, warn};

use super::components::{AuthorityState, GhostReplica, HandoffRequestState};
use super::ghost::apply_ghost_update;
use super::handoff::{finalize_handoff, reject_handoff};
use super::messages::{
    AuthorityEnvelope, GhostUpdate, HandoffAccept, HandoffComplete, HandoffReject, HandoffRequest,
};
use crate::simulation::{Player, TickCounter};

/// Local inbox and outbox for authority traffic.
#[derive(Resource, Default)]
pub struct AuthorityBus {
    pub inbound: VecDeque<AuthorityEnvelope>,
    pub outbound: VecDeque<AuthorityEnvelope>,
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
        &'static mut Transform,
        &'static AuthorityState,
        &'static GhostReplica,
    ),
>;

type HandoffCompleteQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static Player, &'static AuthorityState)>;

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
                bus.pending_ghost_updates.push_back(update);
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
            target_shard_id = request_state.target_shard_id,
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
pub fn apply_ghost_updates(mut bus: ResMut<AuthorityBus>, mut query: GhostUpdateQuery<'_, '_>) {
    while let Some(update) = bus.pending_ghost_updates.pop_front() {
        for (mut transform, state, ghost) in &mut query {
            if matches!(*state, AuthorityState::Ghost) && ghost.source_entity_id == update.entity_id
            {
                trace!(
                    entity_id = update.entity_id,
                    source_shard_id = ghost.source_shard_id,
                    "Applying ghost update"
                );
                apply_ghost_update(&mut transform, update.pos);
                break;
            }
        }
    }
}

/// Completes or rejects pending handoffs.
pub fn complete_handoffs(
    mut bus: ResMut<AuthorityBus>,
    mut commands: Commands,
    query: HandoffCompleteQuery<'_, '_>,
) {
    while let Some(accept) = bus.pending_accepts.pop_front() {
        for (entity, player, state) in &query {
            if player.entity_id == accept.entity_id
                && matches!(*state, AuthorityState::PendingHandoff)
            {
                info!(entity_id = accept.entity_id, "Handoff accepted");
                finalize_handoff(&mut commands, entity);
                break;
            }
        }
    }

    while let Some(reject) = bus.pending_rejects.pop_front() {
        for (entity, player, state) in &query {
            if player.entity_id == reject.entity_id
                && matches!(*state, AuthorityState::PendingHandoff)
            {
                warn!(entity_id = reject.entity_id, "Handoff rejected");
                reject_handoff(&mut commands, entity);
                break;
            }
        }
    }

    while let Some(complete) = bus.pending_completes.pop_front() {
        for (entity, player, state) in &query {
            if player.entity_id == complete.entity_id
                && matches!(*state, AuthorityState::PendingHandoff)
            {
                info!(entity_id = complete.entity_id, "Handoff completed");
                finalize_handoff(&mut commands, entity);
                break;
            }
        }
    }
}

/// Drops outbound packets until a real transport is wired in.
pub fn flush_outbox(mut bus: ResMut<AuthorityBus>) {
    while let Some(_message) = bus.outbound.pop_front() {}
}
