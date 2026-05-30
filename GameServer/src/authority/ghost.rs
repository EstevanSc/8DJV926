use bevy::prelude::*;
use tracing::debug;
use uuid::Uuid;

use super::components::{AuthorityState, GhostReplica};

/// Spawns a read-only ghost entity.
pub fn spawn_ghost_entity(
    commands: &mut Commands,
    entity_id: u32,
    source_shard_id: u32,
    position: Vec2,
) -> Entity {
    debug!(entity_id, source_shard_id, position = ?position, "Spawning ghost entity");
    commands
        .spawn((
            crate::simulation::Player {
                entity_id,
                display_name: format!("ghost:{entity_id}"),
            },
            AuthorityState::Ghost,
            GhostReplica {
                source_shard_id,
                source_entity_id: entity_id,
                source_entity_uuid: Uuid::nil(),
            },
            Transform::from_translation(position.extend(0.0)),
            GlobalTransform::default(),
        ))
        .id()
}

/// Applies a ghost position update.
pub fn apply_ghost_update(transform: &mut Transform, position: Vec2) {
    debug!(position = ?position, "Applying ghost position update");
    transform.translation = position.extend(transform.translation.z);
}
