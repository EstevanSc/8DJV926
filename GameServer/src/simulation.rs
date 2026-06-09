use avian2d::{math::*, prelude::*};
use common::topics::PositionPayload;

use std::collections::HashMap;

use bevy::prelude::*;

use super::net::{SimCommand, SimCommandReceiver};
use super::server::{publish_player_position, BrokerPeer, send_claim_ownership};
use super::char_controller::*;

pub struct SimulationPlugin;

const PLAYER_GRAVITY_SCALE: f32 = 0.0;
const PLAYER_MOVEMENT_ACCELERATION: f32 = 1250.0;
const PLAYER_MOVEMENT_DAMPING: f32 = 5.0;
const PLAYER_COLLIDER_DENSITY: f32 = 2.0;
const FLOOR_RESTITUTION: f32 = 0.0;
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
            .add_message::<SpawnNetEntity>()
            .add_message::<DespawnNetEntity>()
            .add_message::<ClaimAsLocalPlayer>()
            .add_message::<MarkAsGhost>()
            .add_systems(Startup, spawn_floor)
            .add_systems(FixedUpdate, process_net_commands)
            .add_systems(FixedUpdate, (spawn_net_entities).after(process_net_commands))
            .add_systems(FixedUpdate, (claim_ghosts, mark_locals_as_ghosts).after(spawn_net_entities))
            .add_systems(FixedUpdate, despawn_net_entities.after(claim_ghosts))
            .add_systems(FixedUpdate, apply_inputs.after(despawn_net_entities))
            .add_systems(FixedUpdate,publish_entity_positions.after(apply_inputs)
            );
    }
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Latest directional input received from each player this tick.
#[derive(Resource, Default)]
pub struct InputBuffer(pub HashMap<uuid::Uuid, Vec2>);

#[derive(Component)]
pub struct Ghost;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Identifies a player entity on the server.
#[derive(Component, Clone)]
pub struct NetEntity {
    pub connection_id: uuid::Uuid,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Message)]
pub struct SpawnNetEntity {
    pub net_entity: NetEntity,
    pub position: Vec2,
    pub is_ghost: bool,
}

#[derive(Message)]
pub struct DespawnNetEntity {
    pub connection_id: uuid::Uuid,
}

#[derive(Message)]
pub struct ClaimAsLocalPlayer {
    pub connection_id: uuid::Uuid,
    pub speed: [f64; 2],
    pub position: [f64; 2],
}

#[derive(Message)]
pub struct MarkAsGhost {
    pub connection_id: uuid::Uuid,
    pub receiver_shard_id: uuid::Uuid,
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

fn spawn_net_entities(
    mut commands: Commands,
    mut events: MessageReader<SpawnNetEntity>,
) {
    for ev in events.read() {
        if ev.is_ghost {
            commands.spawn((
                Ghost,
                ev.net_entity.clone(),
                Transform::from_translation(ev.position.extend(0.0)),
                GlobalTransform::default(),
                CollisionEventsEnabled,
                CharacterControllerBundle::new(Collider::circle(16.0)).with_movement(PLAYER_MOVEMENT_ACCELERATION, PLAYER_MOVEMENT_DAMPING),
                Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
                ColliderDensity(PLAYER_COLLIDER_DENSITY),
                GravityScale(PLAYER_GRAVITY_SCALE),
            ));
        }
        else {
            commands.spawn((
                ev.net_entity.clone(),
                Transform::from_translation(ev.position.extend(0.0)),
                GlobalTransform::default(),
                CollisionEventsEnabled,
                CharacterControllerBundle::new(Collider::circle(16.0)).with_movement(PLAYER_MOVEMENT_ACCELERATION, PLAYER_MOVEMENT_DAMPING),
                Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
                Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
                ColliderDensity(PLAYER_COLLIDER_DENSITY),
                GravityScale(PLAYER_GRAVITY_SCALE),
            ));
        }
        tracing::info!(
            connection_id = %ev.net_entity.connection_id,
            is_ghost = ev.is_ghost,
            "Spawned Net Entity"
        );
    }
}

fn despawn_net_entities(
    mut commands: Commands,
    mut events: MessageReader<DespawnNetEntity>,
    query: Query<(Entity, &NetEntity)>,
) {
    for ev in events.read() {
        for (entity, net_entity) in &query {
            if net_entity.connection_id == ev.connection_id {
                commands.entity(entity).despawn();
                tracing::info!(connection_id = %ev.connection_id, "Despawned Net Entity");
                break;
            }
        }
    }
}

fn claim_ghosts(
    mut commands: Commands,
    mut events: MessageReader<ClaimAsLocalPlayer>,
    query: Query<(Entity, &NetEntity), With<Ghost>>,
){
    for ev in events.read() {
        for (entity, net_entity) in &query {
            if net_entity.connection_id == ev.connection_id {
                commands.entity(entity).remove::<Ghost>();
                let velocity = Vec2::new(ev.speed[0] as f32, ev.speed[1] as f32);
                commands.entity(entity).insert(LinearVelocity(velocity));
                commands.entity(entity).insert(Transform::from_translation(Vec3::new(ev.position[0] as f32, ev.position[1] as f32, 0.0)));
                
                tracing::info!(connection_id = %ev.connection_id, "Claimed Ghost as Local Player");
                break;
            }
        }
    }
}

fn mark_locals_as_ghosts(
    mut commands: Commands,
    mut events: MessageReader<MarkAsGhost>,
    query: Query<(Entity, &NetEntity, Option<&LinearVelocity>, Option<&Transform>), Without<Ghost>>,
    broker: Option<Res<BrokerPeer>>,
) {
    for ev in events.read() {
        for (entity, net_entity, velocity, transform) in &query {
            if net_entity.connection_id == ev.connection_id {
                commands.entity(entity).insert(Ghost);
                
                let speed = velocity
                    .map(|vel| [vel.x as f64, vel.y as f64])
                    .unwrap_or([0.0, 0.0]);
                
                let position = transform
                    .map(|t| [t.translation.x as f64, t.translation.y as f64])
                    .unwrap_or([0.0, 0.0]);
                
                tracing::info!(connection_id = %ev.connection_id, "Marked Local Player as Ghost");
                
                // Publish the claim ownership message to the new shard so it can get the authority directly
                if let Some(broker) = &broker {
                    send_claim_ownership(
                        broker.as_ref(),
                        ev.receiver_shard_id,
                        ev.connection_id,
                        speed,
                        position,
                    );
                }
                break;
            }
        }
    }
}

fn publish_entity_positions(
    query: Query<(&Transform, &NetEntity), Without<Ghost>>,
    broker: Option<Res<BrokerPeer>>,
) {
    let Some(broker) = broker else {
        return;
    };

    let position_payloads = query
        .iter()
        .map(|(transform, net_entity)| (net_entity.connection_id, PositionPayload {
            position: [
                transform.translation.x as f64,
                transform.translation.y as f64,
            ],
        }))
        .collect::<Vec<(uuid::Uuid, PositionPayload)>>();

    for (connection_id, position_payload) in position_payloads {
        publish_player_position(&broker, connection_id, position_payload);
    }
}

/// Poll the net→sim command channel and translate commands into Bevy messages.
fn process_net_commands(
    cmd_rx: Res<SimCommandReceiver>,
    mut spawn_owned_writer: MessageWriter<SpawnNetEntity>,
    mut despawn_writer: MessageWriter<DespawnNetEntity>,
    mut claim_as_local_writer: MessageWriter<ClaimAsLocalPlayer>,
    mut mark_as_ghost_writer: MessageWriter<MarkAsGhost>,
    mut input_buf: ResMut<InputBuffer>,
    mut query: Query<(Entity, &NetEntity, &mut Transform, Option<&LinearVelocity>)>,
    ghost_query: Query<&NetEntity, With<Ghost>>,
) {
    // Clear every tick so players with no input this tick stop moving.
    input_buf.0.clear();

    let rx = cmd_rx.0.lock().unwrap();
    while let Ok(cmd) = rx.try_recv() {
        match cmd {
            SimCommand::Joined { connection_id, position } => {
                let new_position = Vec2 { x: position.x as f32, y: position.y as f32 };
                spawn_owned_writer.write(SpawnNetEntity {
                    net_entity: NetEntity { connection_id },
                    position: new_position,
                    is_ghost: false,
                });
            }
            SimCommand::GhostJoined { connection_id, position } => {
                let new_position = Vec2 { x: position.x as f32, y: position.y as f32 };
                spawn_owned_writer.write(SpawnNetEntity {
                    net_entity: NetEntity { connection_id},
                    position: new_position,
                    is_ghost: true,
                });
            }
            SimCommand::GhostPositionUpdate { connection_id, position } => {
                let new_position = Vec2 { x: position.x as f32, y: position.y as f32 };
                for (_, net_entity, mut transform, _) in &mut query {
                    if net_entity.connection_id == connection_id {
                        transform.translation = new_position.extend(transform.translation.z);
                        break;
                    }
                }
            }
            SimCommand::Left { connection_id } => {
                despawn_writer.write(DespawnNetEntity { connection_id });
                input_buf.0.remove(&connection_id);
            }
            SimCommand::GhostIsNowLocal { connection_id, speed, position } => {
                let mut found = false;
                for net_entity in &ghost_query{
                    if net_entity.connection_id == connection_id {
                        found = true;
                        println!("ClaimOwnership : Received GhostIsNowLocal for connection_id={}", connection_id);
                        claim_as_local_writer.write(ClaimAsLocalPlayer { connection_id, speed, position });
                        break;
                    }
                }
                if !found {
                    println!("ClaimOwnership : Received GhostIsNowLocal for connection_id={} but no matching Ghost found", connection_id);
                    // create the ghost
                    spawn_owned_writer.write(SpawnNetEntity {
                        net_entity: NetEntity { connection_id},
                        position: Vec2::new(position[0] as f32, position[1] as f32),
                        is_ghost: true,
                    });
                    claim_as_local_writer.write(ClaimAsLocalPlayer { connection_id, speed, position });
                }
            }
            SimCommand::LocalIsNowGhost { connection_id, receiver_shard_id } => {
                println!("ReleaseOwnership : Received LocalIsNowGhost for connection_id={}", connection_id);
                mark_as_ghost_writer.write(MarkAsGhost { connection_id, receiver_shard_id });
            }
            SimCommand::Input { connection_id, dx, dy } => {
                input_buf.0.insert(connection_id, Vec2::new(dx, dy));
            }
        }
    }
}

/// Apply buffered player inputs via the character-controller physics.
fn apply_inputs(
    time: Res<Time>,
    mut query: Query<(&NetEntity, &MovementAcceleration, &mut LinearVelocity), Without<Ghost>>,
    input_buf: Res<InputBuffer>,
) {
    let delta_time = time.delta_secs_f64().adjust_precision();
    
    for (net_entity, movement_acceleration, mut linear_velocity) in &mut query {
        if let Some(&dir) = input_buf.0.get(&net_entity.connection_id) {
            if dir.x != 0.0 {
                linear_velocity.x += dir.x as Scalar * movement_acceleration.0 * delta_time;
            }
            if dir.y != 0.0 {
                linear_velocity.y += dir.y as Scalar * movement_acceleration.0 * delta_time;
            }
        }
    }
}