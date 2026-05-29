use avian2d::{math::*, prelude::*};

use std::collections::HashMap;

use bevy::prelude::*;

use crate::authority::components::AuthorityState;
use common::packets::{PositionBatch, PositionSnapshot};

use super::net::{SimCommand, SimCommandReceiver};
use super::server::{publish_shard_snapshot, BrokerPeer};
use super::char_controller::*;

pub struct SimulationPlugin;

const PLAYER_JUMP_IMPULSE: f32 = 120.0;
const PLAYER_GRAVITY_SCALE: f32 = 4.0;
const PLAYER_MOVEMENT_ACCELERATION: f32 = 1250.0;
const PLAYER_MOVEMENT_DAMPING: f32 = 5.0;
const PLAYER_SLOPE_ANGLE_DEGREES: f32 = 30.0;
const PLAYER_COLLIDER_DENSITY: f32 = 2.0;

const FLOOR_RESTITUTION: f32 = 0.7;
const ARENA_WIDTH: f32 = 10000.0;
const ARENA_WALL_THICKNESS: f32 = 10.0;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_plugins((
            PhysicsPlugins::default().with_length_unit(20.0),
            CharacterControllerPlugin,
            ))
            .init_resource::<TickCounter>()
            .init_resource::<PlayerInputBuffer>()
            .add_message::<SpawnPlayer>()
            .add_message::<DespawnPlayer>()
            .add_systems(Startup, spawn_floor)
            .add_systems(FixedUpdate, process_net_commands)
            .add_systems(FixedUpdate, spawn_players.after(process_net_commands))
            .add_systems(FixedUpdate, despawn_players.after(spawn_players))
            .add_systems(FixedUpdate, apply_player_inputs.after(despawn_players))
            .add_systems(FixedUpdate, publish_shard_snapshots.after(apply_player_inputs));
    }
}

// ---------------------------------------------------------------------------
// Tick counter
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub struct TickCounter(pub u32);

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Latest directional input received from each player this tick.
#[derive(Resource, Default)]
pub struct PlayerInputBuffer(pub HashMap<u32, Vec2>);

type SnapshotQuery<'w, 's> = Query<'w, 's, (&'static Player, &'static AuthorityState, &'static Transform)>;
type InputQuery<'w, 's> = Query<'w, 's, (
    &'static Player,
    &'static AuthorityState,
    &'static MovementAcceleration,
    &'static JumpImpulse,
    &'static mut LinearVelocity,
    Has<Grounded>,
)>;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Identifies a player entity on the server.
#[derive(Component)]
pub struct Player {
    pub entity_id: u32,
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Message)]
pub struct SpawnPlayer {
    pub entity_id: u32,
    pub display_name: String,
    /// World-space spawn position.
    pub position: Vec2,
}

#[derive(Message)]
pub struct DespawnPlayer {
    pub entity_id: u32,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Spawn the static floor so players don't fall into the void.
fn spawn_floor(mut commands: Commands) {
    commands.spawn((
        Transform::from_translation(Vec3::new(0.0, -300.0, 0.0)),
        GlobalTransform::default(),
        RigidBody::Static,
        Collider::rectangle(ARENA_WIDTH, ARENA_WALL_THICKNESS),
        Restitution::new(FLOOR_RESTITUTION).with_combine_rule(CoefficientCombine::Max),
    ));
}

fn spawn_players(
    mut commands: Commands,
    mut events: MessageReader<SpawnPlayer>,
) {
    for ev in events.read() {
        commands.spawn((
            Player {
                entity_id: ev.entity_id,
                display_name: ev.display_name.clone(),
            },
            AuthorityState::Owned,
            Transform::from_translation(ev.position.extend(0.0)),
            GlobalTransform::default(),
            CollisionEventsEnabled,
            CharacterControllerBundle::new(Collider::circle(16.0)).with_movement(PLAYER_MOVEMENT_ACCELERATION, PLAYER_MOVEMENT_DAMPING, PLAYER_JUMP_IMPULSE, (PLAYER_SLOPE_ANGLE_DEGREES as Scalar).to_radians()),
            Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
            Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
            ColliderDensity(PLAYER_COLLIDER_DENSITY),
            GravityScale(PLAYER_GRAVITY_SCALE),
        ));
        tracing::info!(
            entity_id = ev.entity_id,
            name = %ev.display_name,
            "Spawned player"
        );
    }
}

fn despawn_players(
    mut commands: Commands,
    mut events: MessageReader<DespawnPlayer>,
    query: Query<(Entity, &Player)>,
) {
    for ev in events.read() {
        for (entity, player) in &query {
            if player.entity_id == ev.entity_id {
                commands.entity(entity).despawn();
                tracing::info!(entity_id = ev.entity_id, "Despawned player");
                break;
            }
        }
    }
}

/// Every tick: collect positions of all players and broadcast interest-filtered
/// snapshots to each connected client individually.
fn build_position_batch(
    mut tick: ResMut<TickCounter>,
    query: SnapshotQuery<'_, '_>,
) -> Option<PositionBatch> {
    tick.0 = tick.0.wrapping_add(1);

    let snapshots: Vec<PositionSnapshot> = query
        .iter()
        .filter(|(_, authority_state, _)| authority_state.is_snapshot_visible())
        .map(|(player, _, transform)| {
            let position = transform.translation.truncate();
            PositionSnapshot {
                entity_id: player.entity_id,
                display_name: player.display_name.clone(),
                x: position.x as f32,
                y: position.y as f32,
                vx: 0.0,
                vy: 0.0,
            }
        })
        .collect();

    if snapshots.is_empty() {
        return None;
    }

    Some(PositionBatch { tick: tick.0, snapshots })
}

fn publish_shard_snapshots(
    tick: ResMut<TickCounter>,
    query: SnapshotQuery<'_, '_>,
    mut broker: ResMut<BrokerPeer>,
) {
    if let Some(batch) = build_position_batch(tick, query) {
        publish_shard_snapshot(&mut broker, &batch);
    }
}

/// Poll the net→sim command channel and translate commands into Bevy messages.
fn process_net_commands(
    cmd_rx: Res<SimCommandReceiver>,
    mut spawn_writer: MessageWriter<SpawnPlayer>,
    mut despawn_writer: MessageWriter<DespawnPlayer>,
    mut input_buf: ResMut<PlayerInputBuffer>,
) {
    // Clear every tick so players with no input this tick stop moving.
    input_buf.0.clear();

    let rx = cmd_rx.0.lock().unwrap();
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            SimCommand::Joined { entity_id, display_name } => {
                spawn_writer.write(SpawnPlayer {
                    entity_id,
                    display_name,
                    position: Vec2::ZERO,
                });
            }
            SimCommand::Left { entity_id } => {
                despawn_writer.write(DespawnPlayer { entity_id });
                input_buf.0.remove(&entity_id);
            }
            SimCommand::Input { entity_id, dx, dy } => {
                input_buf.0.insert(entity_id, Vec2::new(dx, dy));
            }
        }
    }
}

/// Apply buffered player inputs via the character-controller physics.
fn apply_player_inputs(
    time: Res<Time>,
    mut query: InputQuery<'_, '_>,
    input_buf: Res<PlayerInputBuffer>,
) {
    let delta_time = time.delta_secs_f64().adjust_precision();

    for (player, authority_state, movement_acceleration, jump_impulse, mut linear_velocity, is_grounded) in &mut query {
        if !authority_state.allows_local_simulation() {
            continue;
        }

        if let Some(&dir) = input_buf.0.get(&player.entity_id) {
            if dir.x != 0.0 {
                linear_velocity.x += dir.x as Scalar * movement_acceleration.0 * delta_time;
            }
            if dir.y > 0.5 && is_grounded {
                linear_velocity.y = jump_impulse.0;
            }
        }
    }
}
