use avian2d::{math::*, prelude::*};
use common::topics::PositionPayload;

use std::collections::HashMap;

use bevy::prelude::*;

use super::net::{SimCommand, SimCommandReceiver};
use super::server::{publish_player_position};
use super::char_controller::*;

pub struct SimulationPlugin;

const PLAYER_JUMP_IMPULSE: f32 = 120.0;
const PLAYER_GRAVITY_SCALE: f32 = 0.0;
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
            .init_resource::<InputBuffer>()
            .add_message::<SpawnGameplayEntity>()
            .add_message::<DespawnGameplayEntity>()
            .add_systems(Startup, spawn_floor)
            .add_systems(FixedUpdate, process_net_commands)
            .add_systems(FixedUpdate, (spawn_gameplay_entities).after(process_net_commands))
            .add_systems(FixedUpdate, despawn_gameplay_entities.after(spawn_gameplay_entities))
            .add_systems(FixedUpdate, apply_inputs.after(despawn_gameplay_entities))
            .add_systems(FixedUpdate,publish_entity_positions.after(apply_inputs)
            );
    }
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Latest directional input received from each player this tick.
#[derive(Resource, Default)]
pub struct InputBuffer(pub HashMap<u32, Vec2>);

#[derive(Component)]
pub struct Ghost;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Identifies a player entity on the server.
#[derive(Component, Clone)]
pub struct GameplayEntity {
    pub entity_id: u32,
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Message)]
pub struct SpawnGameplayEntity {
    pub gameplay_entity: GameplayEntity,
    pub position: Vec2,
    pub is_ghost: bool,
}

#[derive(Message)]
pub struct DespawnGameplayEntity {
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

fn spawn_gameplay_entities(
    mut commands: Commands,
    mut events: MessageReader<SpawnGameplayEntity>,
) {
    for ev in events.read() {
        if ev.is_ghost {
            commands.spawn((
                Ghost,
                ev.gameplay_entity.clone(),
                Transform::from_translation(ev.position.extend(0.0)),
                GlobalTransform::default(),
                CollisionEventsEnabled,
                CharacterControllerBundle::new(Collider::circle(16.0)).with_movement(PLAYER_MOVEMENT_ACCELERATION, PLAYER_MOVEMENT_DAMPING, PLAYER_JUMP_IMPULSE, (PLAYER_SLOPE_ANGLE_DEGREES as Scalar).to_radians()),
                Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
                ColliderDensity(PLAYER_COLLIDER_DENSITY),
                GravityScale(PLAYER_GRAVITY_SCALE),
            ));
        }
        else {
            commands.spawn((
                ev.gameplay_entity.clone(),
                Transform::from_translation(ev.position.extend(0.0)),
                GlobalTransform::default(),
                CollisionEventsEnabled,
                CharacterControllerBundle::new(Collider::circle(16.0)).with_movement(PLAYER_MOVEMENT_ACCELERATION, PLAYER_MOVEMENT_DAMPING, PLAYER_JUMP_IMPULSE, (PLAYER_SLOPE_ANGLE_DEGREES as Scalar).to_radians()),
                Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
                ColliderDensity(PLAYER_COLLIDER_DENSITY),
                GravityScale(PLAYER_GRAVITY_SCALE),
            ));
        }
        tracing::info!(
            entity_id = ev.gameplay_entity.entity_id,
            name = %ev.gameplay_entity.display_name,
            is_ghost = ev.is_ghost,
            "Spawned Gameplay Entity"
        );
    }
}

fn despawn_gameplay_entities(
    mut commands: Commands,
    mut events: MessageReader<DespawnGameplayEntity>,
    query: Query<(Entity, &GameplayEntity)>,
) {
    for ev in events.read() {
        for (entity, gameplay_entity) in &query {
            if gameplay_entity.entity_id == ev.entity_id {
                commands.entity(entity).despawn();
                tracing::info!(entity_id = ev.entity_id, "Despawned Gameplay Entity");
                break;
            }
        }
    }
}

fn publish_entity_positions(
    query: Query<&Transform, Without<Ghost>>,
) {
    let position_payloads = query.iter().enumerate().map(|(i, transform)| {
    PositionPayload {
        entity_id: i as u32,
        position: [transform.translation.x as f64, transform.translation.y as f64],

        }
    }).collect::<Vec<_>>();

    for snapshot in position_payloads {
        publish_player_position(snapshot);
    }
}

/// Poll the net→sim command channel and translate commands into Bevy messages.
fn process_net_commands(
    cmd_rx: Res<SimCommandReceiver>,
    mut spawn_owned_writer: MessageWriter<SpawnGameplayEntity>,
    mut despawn_writer: MessageWriter<DespawnGameplayEntity>,
    mut input_buf: ResMut<InputBuffer>,
    mut query: Query<(Entity, &GameplayEntity, &mut Transform, Option<&LinearVelocity>)>,
) {
    // Clear every tick so players with no input this tick stop moving.
    input_buf.0.clear();

    let rx = cmd_rx.0.lock().unwrap();
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            SimCommand::Joined { entity_id, display_name, position } => {
                let new_position = Vec2 { x: position.x as f32, y: position.y as f32 };
                
                spawn_owned_writer.write(SpawnGameplayEntity {
                    gameplay_entity: GameplayEntity { entity_id, display_name },
                    position: new_position,
                    is_ghost: false,
                });
            }
            SimCommand::GhostJoined { client_id, entity_id, position } => {
                let new_position = Vec2 { x: position.x as f32, y: position.y as f32 };
                spawn_owned_writer.write(SpawnGameplayEntity {
                    gameplay_entity: GameplayEntity { entity_id, display_name: client_id.to_string() },
                    position: new_position,
                    is_ghost: true,
                });
            }
            SimCommand::GhostPositionUpdate { entity_id, position } => {
                let new_position = Vec2 { x: position.x as f32, y: position.y as f32 };
                for (_, gameplay_entity, mut transform, _) in &mut query {
                    if gameplay_entity.entity_id == entity_id {
                        transform.translation = new_position.extend(transform.translation.z);
                        break;
                    }
                }
            }
            SimCommand::Left { entity_id } => {
                despawn_writer.write(DespawnGameplayEntity { entity_id });
                input_buf.0.remove(&entity_id);
            }
            SimCommand::Input { entity_id, dx, dy } => {
                input_buf.0.insert(entity_id, Vec2::new(dx, dy));
            }
        }
    }
}

/// Apply buffered player inputs via the character-controller physics.
fn apply_inputs(
    time: Res<Time>,
    mut query: Query<(&GameplayEntity, &MovementAcceleration, &mut LinearVelocity), Without<Ghost>>,
    input_buf: Res<InputBuffer>,
) {
    let delta_time = time.delta_secs_f64().adjust_precision();
    
    for (gameplay_entity, movement_acceleration, mut linear_velocity) in &mut query {
        if let Some(&dir) = input_buf.0.get(&gameplay_entity.entity_id) {
            if dir.x != 0.0 {
                linear_velocity.x += dir.x as Scalar * movement_acceleration.0 * delta_time;
            }
            if dir.y != 0.0 {
                linear_velocity.y += dir.y as Scalar * movement_acceleration.0 * delta_time;
            }
        }
    }
}