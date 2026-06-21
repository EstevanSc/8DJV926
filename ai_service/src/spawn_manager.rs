use std::time::Duration;

use bevy::prelude::*;
use bevy_behave::prelude::*;
use rand::RngExt;
use uuid::Uuid;

use crate::behaviour::trees::build_tree;
use crate::bridge::QuadtreeBoundaries;
use crate::components::{AiEntity, AiIntent, AiPath, AiPosition, AiStats, PatrolRoute, Perception};
use crate::config::Config;

/// Bevy plugin that manages dynamic AI spawns based on Quadtree limits.
pub struct SpawnPlugin;

impl Plugin for SpawnPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_spawn_timer)
            .add_systems(Update, tick_respawns);
    }
}

/// Tracks the universal respawn cadence based on configuration.
#[derive(Resource)]
pub struct SpawnTimer(pub Timer);

fn setup_spawn_timer(mut commands: Commands, config: Res<Config>) {
    commands.insert_resource(SpawnTimer(Timer::new(
        Duration::from_secs_f32(config.spawn_frequency_secs),
        TimerMode::Repeating,
    )));
}

fn tick_respawns(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<Config>,
    mut timer: ResMut<SpawnTimer>,
    boundaries: Option<Res<QuadtreeBoundaries>>,
    ai_query: Query<(), With<AiEntity>>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let current_ai = ai_query.iter().count();
    if current_ai >= config.max_ai {
        return;
    }

    let Some(boundaries) = boundaries else { return };
    if boundaries.0.is_empty() {
        return;
    }

    // Calcul des limites globales (bords du monde)
    let mut global_min_x = f32::MAX;
    let mut global_max_x = f32::MIN;
    let mut global_min_y = f32::MAX;
    let mut global_max_y = f32::MIN;

    for b in &boundaries.0 {
        global_min_x = global_min_x.min((b.x - b.half_size) as f32);
        global_max_x = global_max_x.max((b.x + b.half_size) as f32);
        global_min_y = global_min_y.min((b.y - b.half_size) as f32);
        global_max_y = global_max_y.max((b.y + b.half_size) as f32);
    }

    let mut sorted_boundaries = boundaries.0.clone();
    // Sort descending by size (area proportional to half_size)
    sorted_boundaries.sort_by(|a, b| {
        b.half_size
            .partial_cmp(&a.half_size)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let pct = config.spawn_top_shard_percentage.clamp(0.0, 100.0) / 100.0;
    let top_count = (sorted_boundaries.len() as f32 * pct).ceil() as usize;
    let top_count = top_count.clamp(1, sorted_boundaries.len());

    let mut rng = rand::rng();
    let chosen_idx = rng.random_range(0..top_count);
    let chosen_shard = &sorted_boundaries[chosen_idx];

    let mut min_x = (chosen_shard.x - chosen_shard.half_size) as f32;
    let mut max_x = (chosen_shard.x + chosen_shard.half_size) as f32;
    let mut min_y = (chosen_shard.y - chosen_shard.half_size) as f32;
    let mut max_y = (chosen_shard.y + chosen_shard.half_size) as f32;

    let padding = config.spawn_padding as f32;

    // Apply padding only if the shard edge coincides with the global edge, to avoid pushing spawns outside the world bounds
    if (min_x - global_min_x).abs() < 1e-3 {
        min_x += padding;
    }
    if (global_max_x - max_x).abs() < 1e-3 {
        max_x -= padding;
    }
    if (min_y - global_min_y).abs() < 1e-3 {
        min_y += padding;
    }
    if (global_max_y - max_y).abs() < 1e-3 {
        max_y -= padding;
    }

    // fallback if padding made the spawn area invalid
    if min_x > max_x {
        min_x = chosen_shard.x as f32;
        max_x = chosen_shard.x as f32;
    }
    if min_y > max_y {
        min_y = chosen_shard.y as f32;
        max_y = chosen_shard.y as f32;
    }

    let spawn_x = rng.random_range(min_x..=max_x);
    let spawn_y = rng.random_range(min_y..=max_y);

    // Generate random consecutive patrol waypoints around the spawn point
    let mut waypoints = vec![[spawn_x, spawn_y]];
    for _ in 0..4 {
        let wp_x = waypoints.last().unwrap()[0] + rng.random_range(-200.0..=200.0);
        let wp_y = waypoints.last().unwrap()[1] + rng.random_range(-200.0..=200.0);
        waypoints.push([wp_x, wp_y]);
    }

    let id = Uuid::new_v4();
    let agent = commands
        .spawn(AiEntity { id })
        .insert(AiPosition {
            x: spawn_x,
            y: spawn_y,
        })
        .insert(Perception::default())
        .insert(AiPath {
            waypoints: Vec::new(),
        })
        .insert(PatrolRoute {
            waypoints,
            current: 0,
        })
        .insert(AiIntent::Idle)
        .insert(AiStats {
            health: 100,
            max_health: 100,
            mana: 50,
        })
        .id();

    let tree = build_tree(id);
    commands
        .spawn((Name::new(format!("BT-{id}")), tree))
        .insert(BehaveTargetEntity::Entity(agent));

    tracing::info!(
        "Spawned AI {} at ({:.1}, {:.1}) in a top {}% shard (half_size: {:.1})",
        id,
        spawn_x,
        spawn_y,
        config.spawn_top_shard_percentage,
        chosen_shard.half_size
    );
}
