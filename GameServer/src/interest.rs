use bevy::prelude::*;

use common::constants::INTEREST_RADIUS_TILES;
use common::packets::PositionSnapshot;

/// Tile size in world units — determines the interest bubble radius.
const TILE_SIZE: f32 = 32.0;
const INTEREST_RADIUS: f32 = INTEREST_RADIUS_TILES as f32 * TILE_SIZE;

/// Return position snapshots of all entities visible from `observer_pos`.
/// Only entities within `INTEREST_RADIUS` world units are included.
pub fn interest_query(
    observer_pos: Vec2,
    all: &[(u32, String, Vec2)], // (entity_id, display_name, position)
) -> Vec<PositionSnapshot> {
    let radius_sq = INTEREST_RADIUS * INTEREST_RADIUS;
    all.iter()
        .filter(|(_, _, pos)| (*pos - observer_pos).length_squared() <= radius_sq)
        .map(|(entity_id, name, pos)| PositionSnapshot {
            entity_id: *entity_id,
            display_name: name.clone(),
            x: pos.x,
            y: pos.y,
            vx: 0.0,
            vy: 0.0,
        })
        .collect()
}
