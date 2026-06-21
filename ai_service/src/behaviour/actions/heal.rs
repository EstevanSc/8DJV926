use crate::components::{AiIntent, AiStats};
use bevy::prelude::*;
use bevy_behave::prelude::*;
use common::ability_type::AbilityType;

#[derive(Clone, Debug)]
pub struct CheckLowHealth;

#[derive(Clone, Debug)]
pub struct CastHeal;

/// Check if health is below 50%
pub fn on_check_low_health(
    trigger: On<BehaveTrigger<CheckLowHealth>>,
    query: Query<&AiStats>,
    mut commands: Commands,
) {
    let ctx = trigger.event().ctx();
    if let Ok(stats) = query.get(ctx.target_entity()) {
        if stats.health < (stats.max_health / 2) {
            commands.trigger(ctx.success());
            return;
        }
    }
    commands.trigger(ctx.failure());
}

/// Cast Heal ability
pub fn on_cast_heal(
    trigger: On<BehaveTrigger<CastHeal>>,
    mut query: Query<&mut AiIntent>,
    mut commands: Commands,
) {
    let ctx = trigger.event().ctx();
    if let Ok(mut intent) = query.get_mut(ctx.target_entity()) {
        *intent = AiIntent::CastAbility(AbilityType::Heal, None);
        commands.trigger(ctx.success());
    }
}
