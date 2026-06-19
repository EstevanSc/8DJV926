use bevy::prelude::*;
use uuid::Uuid;

/// Marks an entity as an AI-controlled NPC and holds its broker identity.
#[derive(Component, Debug, Clone)]
pub struct AiEntity {
    pub id: Uuid,
}

/// Current world position of the AI, updated from EntityPositionUpdate broadcasts.
#[derive(Component, Debug, Clone, Default)]
pub struct AiPosition {
    pub x: f32,
    pub y: f32,
}

// Current path of the AI, as a list of waypoints. Updated from PathResponse messages.
#[derive(Component, Debug, Clone, Default)]
pub struct AiPath {
    pub waypoints: Vec<[f32; 2]>,
}

/// Nearby entities received via AOI broadcasts from the spatial service.
#[derive(Component, Debug, Clone, Default)]
pub struct Perception {
    pub nearby: Vec<(Uuid, [f32; 2])>,
}

/// Waypoints for a patrol route. The AI cycles through them in order.
#[derive(Component, Debug, Clone)]
pub struct PatrolRoute {
    pub waypoints: Vec<[f32; 2]>,
    pub current: usize,
}

impl PatrolRoute {
    /// Returns the current target waypoint.
    pub fn target(&self) -> [f32; 2] {
        self.waypoints[self.current]
    }

    /// Advances to the next waypoint, wrapping around.
    pub fn advance(&mut self) {
        self.current = (self.current + 1) % self.waypoints.len();
    }
}

/// Intent produced by the behaviour tree, consumed by the bridge to send broker messages.
#[derive(Component, Debug, Clone)]
pub enum AiIntent {
    MoveTo([f32; 2]),
    CastAbility(common::ability_type::AbilityType, Option<[f32; 2]>),
    Idle,
}

#[derive(Component, Debug, Clone, Default)]
pub struct AiStats {
    pub health: i32,
    pub max_health: i32,
    pub mana: i32,
}