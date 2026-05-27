#![allow(dead_code, unused_imports)]

pub mod components;
pub mod ghost;
pub mod handoff;
pub mod messages;
pub mod systems;

#[cfg(test)]
pub mod tests;

use bevy::prelude::*;
use tracing::info;

/// Bevy plugin for the authority subsystem.
pub struct AuthorityPlugin;

impl Plugin for AuthorityPlugin {
    /// Registers authority resources and systems.
    fn build(&self, app: &mut App) {
        app.init_resource::<components::AuthorityConfig>()
            .init_resource::<systems::AuthorityBus>()
            .add_systems(Startup, log_authority_startup)
            .add_systems(FixedUpdate, systems::route_inbound_messages)
            .add_systems(
                FixedUpdate,
                systems::apply_handoff_requests.after(systems::route_inbound_messages),
            )
            .add_systems(
                FixedUpdate,
                systems::apply_ghost_updates.after(systems::route_inbound_messages),
            )
            .add_systems(
                FixedUpdate,
                systems::complete_handoffs.after(systems::route_inbound_messages),
            );
    }
}

/// Logs the local authority configuration once at startup.
fn log_authority_startup(config: Res<components::AuthorityConfig>) {
    info!(
        local_shard_id = config.local_shard_id,
        handoff_margin = config.handoff_margin,
        "Authority subsystem initialized"
    );
}

pub use components::{AuthorityConfig, AuthorityState, GhostReplica, HandoffRequestState};
pub use ghost::{apply_ghost_update, spawn_ghost_entity};
pub use handoff::{begin_handoff, build_handoff_request, finalize_handoff, reject_handoff};
pub use messages::{
    AuthorityEnvelope, GhostUpdate, HandoffAccept, HandoffComplete, HandoffReject, HandoffRequest,
};
