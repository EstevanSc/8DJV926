use bevy::prelude::*;
use bevy_behave::prelude::*;
use crate::components::{AiPosition, Perception, AiIntent};
use common::ability_type::AbilityType;

const FIREBALL_RANGE_SQ: f32 = 100.0 * 100.0;

#[derive(Clone, Debug)]
pub struct CheckAggroDistance;

#[derive(Clone, Debug)]
pub struct CastFireball;

/// Check if any nearby target is within fireball range
pub fn on_check_aggro_distance(
    trigger: On<BehaveTrigger<CheckAggroDistance>>,
    query: Query<(&AiPosition, &Perception)>,
    mut commands: Commands,
) {
    let ctx = trigger.event().ctx();
    if let Ok((pos, perception)) = query.get(ctx.target_entity()) {
        if perception.nearby.iter().any(|(_, target_pos)| {
            let dist_sq = (target_pos[0] - pos.x).powi(2) + (target_pos[1] - pos.y).powi(2);
            dist_sq < FIREBALL_RANGE_SQ
        }) {
            commands.trigger(ctx.success());
            return;
        }
    }
    commands.trigger(ctx.failure());
}

/// Cast Fireball on the first nearby target
pub fn on_cast_fireball(
    trigger: On<BehaveTrigger<CastFireball>>,
    mut query: Query<(&AiPosition, &Perception, &mut AiIntent)>,
    mut commands: Commands,
) {
    let ctx = trigger.event().ctx();
    if let Ok((pos, perception, mut intent)) = query.get_mut(ctx.target_entity()) {
        // Vise le premier ennemi proche
        if let Some((_, target_pos)) = perception.nearby.first() {
            let dir = [target_pos[0] - pos.x, target_pos[1] - pos.y];
            *intent = AiIntent::CastAbility(AbilityType::Fireball, Some(dir));
            commands.trigger(ctx.success());
            return;
        }
    }
    commands.trigger(ctx.failure());
}