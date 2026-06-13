use bevy::prelude::*;
use bevy_behave::prelude::*;

use crate::components::{AiIntent, AiPosition, PatrolRoute};

/// Trigger payload for the Patrol behaviour tree node.
#[derive(Clone, Debug)]
pub struct Patrol;

/// Observer that handles Patrol trigger events and updates AiIntent.
/// Responds with success each tick so the BT keeps re-evaluating.
pub fn on_patrol(
    trigger: On<BehaveTrigger<Patrol>>,
    mut query: Query<(&AiPosition, &mut PatrolRoute, &mut AiIntent)>,
    mut commands: Commands,
) {
    print!("Patrol triggered for entity {:?}", trigger.event().ctx().target_entity());
    let ctx = trigger.event().ctx();

    let Ok((pos, mut route, mut intent)) = query.get_mut(ctx.target_entity()) else {
        commands.trigger(ctx.failure());
        return;
    };

    let wp = route.target();
    let dist = ((wp[0] - pos.x).powi(2) + (wp[1] - pos.y).powi(2)).sqrt();

    if dist < 1.0 {
        route.advance();
    }

    *intent = AiIntent::MoveTo(route.target());
    commands.trigger(ctx.success());
}