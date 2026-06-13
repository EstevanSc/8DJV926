use std::time::Duration;

use bevy::prelude::*;
use uuid::Uuid;

use crate::behaviour::trees::build_tree;
use crate::components::{AiEntity, AiIntent, AiPosition, PatrolRoute, Perception};
use crate::config::{Config, ZoneConfig};

/// Bevy plugin that manages AI spawn zones from config.
pub struct SpawnPlugin;

impl Plugin for SpawnPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_zones)
            .add_systems(Update, tick_respawns);
    }
}

/// Tracks the respawn timer for a zone.
#[derive(Component)]
pub struct ZoneRespawn {
    pub zone: ZoneConfig,
    pub timer: Timer,
}

fn setup_zones(mut commands: Commands, config: Res<Config>) {
    for zone in &config.zones {
        spawn_zone_entities(&mut commands, zone);
        commands.spawn(ZoneRespawn {
            zone: zone.clone(),
            timer: Timer::new(
                Duration::from_secs(zone.respawn_delay_secs),
                TimerMode::Repeating,
            ),
        });
    }
}

fn tick_respawns(
    mut commands: Commands,
    time: Res<Time>,
    mut zones: Query<&mut ZoneRespawn>,
) {
    for mut zone in &mut zones {
        if zone.timer.tick(time.delta()).just_finished() {
            let config = zone.zone.clone();
            spawn_zone_entities(&mut commands, &config);
        }
    }
}

/// Spawns Bevy entities for a zone.
fn spawn_zone_entities(commands: &mut Commands, zone: &ZoneConfig) {
    for i in 0..zone.count {
        let id = Uuid::new_v4();
        let offset = i as f64 * 5.0;
        let pos = [zone.center[0] + offset, zone.center[1]];

        let waypoints = vec![
            pos,
            [pos[0] + 20.0, pos[1]],
            [pos[0] + 20.0, pos[1] + 20.0],
            [pos[0], pos[1] + 20.0],
        ];

        let tree = build_tree(id);

        commands.spawn(AiEntity { id })
            .insert(AiPosition { x: pos[0], y: pos[1] })
            .insert(Perception::default())
            .insert(PatrolRoute { waypoints, current: 0 })
            .insert(AiIntent::Idle)
            .insert(tree);
    }
}