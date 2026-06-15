use bevy::prelude::*;
use bevy_behave::prelude::*;

use crate::components::{AiIntent, AiPosition, Perception};

const AGGRO_RANGE: f64 = 300.0;

/// Trigger payload for the CheckNearby condition node.
#[derive(Clone, Debug)]
pub struct CheckNearby;

/// Trigger payload for the Chase behaviour tree node.
#[derive(Clone, Debug)]
pub struct Chase;

/// Observer that checks if any entity is within aggro range.
pub fn on_check_nearby(
    trigger: On<BehaveTrigger<CheckNearby>>,
    query: Query<&Perception>,
    mut commands: Commands,
) {
    tracing::debug!("CheckNearby triggered for entity {:?}", trigger.event().ctx().target_entity());
    tracing::debug!("Perception query: {:?}", query.iter().collect::<Vec<_>>());
    let ctx = trigger.event().ctx();

    let in_range = query
        .get(ctx.target_entity())
        .map(|p| p.nearby.iter().any(|(_, pos)| pos[0].powi(2) + pos[1].powi(2) < AGGRO_RANGE.powi(2)))
        .unwrap_or(false);

    if in_range {
        commands.trigger(ctx.success());
    } else {
        commands.trigger(ctx.failure());
    }
}

/// Observer that handles Chase trigger events and updates AiIntent.
/// Responds with success each tick so the BT keeps re-evaluating.
pub fn on_chase(
    trigger: On<BehaveTrigger<Chase>>,
    mut query: Query<(&AiPosition, &Perception, &mut AiIntent)>,
    mut commands: Commands,
) {
    tracing::debug!("Chase triggered for entity {:?}", trigger.event().ctx().target_entity());
    let ctx = trigger.event().ctx();

    let Ok((pos, perception, mut intent)) = query.get_mut(ctx.target_entity()) else {
        commands.trigger(ctx.failure());
        return;
    };

    let nearest = perception.nearby.iter().min_by(|a, b| {
        dist2(pos, a.1).partial_cmp(&dist2(pos, b.1)).unwrap()
    });

    match nearest {
        Some((_, target_pos)) => {
            *intent = AiIntent::MoveTo(*target_pos);
            commands.trigger(ctx.success());
        }
        None => {
            commands.trigger(ctx.failure());
        }
    }
}

fn dist2(pos: &AiPosition, target: [f64; 2]) -> f64 {
    (target[0] - pos.x).powi(2) + (target[1] - pos.y).powi(2)
}