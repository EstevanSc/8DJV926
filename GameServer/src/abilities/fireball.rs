use crate::simulation::AbilityHitEntity;
use avian2d::{math::*, prelude::*};
use bevy::prelude::*;
use common::ability_type::AbilityType;

pub struct FireballPlugin;

impl Plugin for FireballPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, handle_fireball_collisions);
    }
}

#[derive(Component)]
pub struct Fireball;

#[derive(Component)]
pub struct Caster(pub Entity);

#[derive(Component)]
pub struct Speed(pub Scalar);

#[derive(Component)]
pub struct Direction(pub Vector);

#[derive(Bundle)]
pub struct FireballBundle {
    pub fireball: Fireball,
    pub caster: Caster,
    pub speed: Speed,
    pub direction: Direction,
    pub body: RigidBody,
    pub collider: Collider,
    pub velocity: LinearVelocity,
    pub friction: Friction,
    pub restitution: Restitution,
    pub collision_events: CollisionEventsEnabled,
}

impl FireballBundle {
    pub fn new(caster: Entity, direction: Vec2, speed: f32) -> Self {
        let velocity_vector = direction.normalize_or_zero() * speed as Scalar;

        Self {
            fireball: Fireball,
            caster: Caster(caster),
            speed: Speed(speed as Scalar),
            direction: Direction(direction),
            body: RigidBody::Dynamic,
            collider: Collider::circle(8.0),
            velocity: LinearVelocity(velocity_vector),
            friction: Friction::ZERO,
            restitution: Restitution::ZERO,
            collision_events: CollisionEventsEnabled,
        }
    }
}

pub fn handle_fireball_collisions(
    mut commands: Commands,
    mut collision_events: MessageReader<CollisionStart>,
    mut hit_writer: MessageWriter<AbilityHitEntity>,
    fireball_query: Query<(Entity, &Caster, &Direction, &Transform), With<Fireball>>,
    collidable_query: Query<Entity>,
) {
    for collision in collision_events.read() {
        // Check if either entity in the collision is a fireball
        let fireball_data = fireball_query
            .get(collision.collider1)
            .map(|data| (collision.collider1, data))
            .or_else(|_| {
                fireball_query
                    .get(collision.collider2)
                    .map(|data| (collision.collider2, data))
            });

        if let Ok((fireball_entity, (_fb_ent, caster, _direction, transform))) = fireball_data {
            let hit_entity = if fireball_entity == collision.collider1 {
                collision.collider2
            } else {
                collision.collider1
            };

            // Prevent the fireball from blowing up on the person who cast it
            if hit_entity == caster.0 {
                continue;
            }

            // Ignore collisions inside the 500x500 safe zone centered at (0, 0)
            let pos = transform.translation;
            if pos.x.abs() <= 250.0 && pos.y.abs() <= 250.0 {
                continue;
            }

            // Verify the hit entity is something valid we can collide with
            if collidable_query.get(hit_entity).is_ok() {
                // Only deal damage/write hit event if outside the 500x500 safe zone centered at (0, 0)
                let pos = transform.translation;
                if pos.x.abs() <= 250.0 && pos.y.abs() <= 250.0 {
                    tracing::info!(
                        "Fireball {:?} hit Entity {:?} inside safe zone (ignored damage)",
                        fireball_entity,
                        hit_entity
                    );
                } else {
                    // Send ability_hit_entity event
                    hit_writer.write(AbilityHitEntity {
                        caster: caster.0,
                        hit_entity,
                        ability_type: AbilityType::Fireball,
                    });

                    tracing::info!("Fireball {:?} hit Entity {:?}", fireball_entity, hit_entity);
                }
            }
        }
    }
}
