use std::collections::VecDeque;

use bevy::prelude::*;
use uuid::Uuid;

use crate::authority::components::{AuthorityState, GhostReplica};
use crate::authority::handoff::{begin_handoff, finalize_handoff, reject_handoff};
use crate::authority::systems::{complete_handoffs, route_inbound_messages, AuthorityBus};
use crate::authority::{
    AuthorityEnvelope, HandoffAccept, HandoffComplete, HandoffReject, HandoffRequest,
};
use crate::simulation::Player;

/// Holds the entity under test.
#[derive(Resource, Clone, Copy)]
struct TestEntity(Entity);

/// Tracks the handoff transition stage.
#[derive(Resource, Clone, Copy, Default)]
struct TransitionStage(u8);

/// Builds a sample handoff request for testing.
fn sample_request(entity_id: u32) -> HandoffRequest {
    HandoffRequest {
        entity_id,
        pos: Vec2::new(10.0, 20.0),
        vel: Vec2::new(30.0, 40.0),
        state: [entity_id as u8; 64],
    }
}

/// Verifies the Owned, PendingHandoff, Ghost, Owned cycle.
fn setup_pending_handoff(mut commands: Commands) {
    let entity = commands
        .spawn((
            Player {
                entity_id: 100,
                display_name: "test-player".to_string(),
            },
            AuthorityState::Owned,
        ))
        .id();
    begin_handoff(&mut commands, entity, Uuid::new_v4(), sample_request(100), 12);
    commands.insert_resource(TestEntity(entity));
    commands.insert_resource(TransitionStage::default());
}

/// Advances the handoff transition through its stages.
fn advance_transition(
    mut commands: Commands,
    entity: Res<TestEntity>,
    mut stage: ResMut<TransitionStage>,
) {
    match stage.0 {
        1 => {
            commands
                .entity(entity.0)
                .insert((AuthorityState::Ghost, GhostReplica {
                    source_shard_id: 9,
                    source_entity_id: 100,
                    source_entity_uuid: Uuid::new_v4(),
                }));
            stage.0 = 2;
        }
        2 => {
            finalize_handoff(&mut commands, entity.0);
            stage.0 = 3;
        }
        _ => {}
    }
}

/// Verifies the Owned -> PendingHandoff -> Ghost -> Owned cycle.
#[test]
fn authority_state_transitions_cycle() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_systems(Startup, setup_pending_handoff);
    app.add_systems(Update, advance_transition);

    app.update();

    let entity = app.world().resource::<TestEntity>().0;
    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::PendingHandoff));

    app.world_mut().resource_mut::<TransitionStage>().0 = 1;
    app.update();

    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::Ghost));
    assert!(app.world().entity(entity).contains::<GhostReplica>());

    app.world_mut().resource_mut::<TransitionStage>().0 = 2;
    app.update();

    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::Owned));
    assert!(!app.world().entity(entity).contains::<GhostReplica>());
    assert!(!app.world().entity(entity).contains::<crate::authority::HandoffRequestState>());
}

fn spawn_ghost(mut commands: Commands) {
    let entity = crate::authority::spawn_ghost_entity(&mut commands, 100, 13, Vec2::new(1.0, 2.0));
    commands.insert_resource(TestEntity(entity));
}

/// Verifies ghost position updates are applied correctly.
#[test]
fn ghost_replication_update_correctness() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_systems(Startup, spawn_ghost);

    app.update();

    let entity = app.world().resource::<TestEntity>().0;
    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::Ghost));

    {
        let mut entity_mut = app.world_mut().entity_mut(entity);
        let mut transform = entity_mut
            .get_mut::<Transform>()
            .expect("ghost should have a transform");
        crate::authority::apply_ghost_update(&mut transform, Vec2::new(9.0, 10.0));
    }

    let translation = app.world().entity(entity).get::<Transform>().unwrap().translation.truncate();
    assert_eq!(translation, Vec2::new(9.0, 10.0));
}

/// Verifies a handoff accept is stored until completion.
#[test]
fn handoff_accept_waits_for_completion() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<AuthorityBus>();
    app.add_systems(Startup, setup_pending_handoff);
    app.add_systems(Update, route_inbound_messages);
    app.add_systems(Update, complete_handoffs.after(route_inbound_messages));

    app.update();

    let entity = app.world().resource::<TestEntity>().0;
    let mut bus = app.world_mut().resource_mut::<AuthorityBus>();
    bus.inbound.push_back(AuthorityEnvelope::HandoffAccept(HandoffAccept { entity_id: 100 }));
    drop(bus);

    app.update();

    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::PendingHandoff));
    assert!(app
        .world()
        .entity(entity)
        .get::<crate::authority::HandoffRequestState>()
        .is_some_and(|state| state.accepted));

    let mut bus = app.world_mut().resource_mut::<AuthorityBus>();
    bus.inbound.push_back(AuthorityEnvelope::HandoffComplete(HandoffComplete { entity_id: 100 }));
    drop(bus);

    app.update();

    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::Owned));
    assert!(!app.world().entity(entity).contains::<crate::authority::HandoffRequestState>());
}

/// Verifies a handoff reject restores Owned state.
#[test]
fn handoff_reject_restores_owned() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<AuthorityBus>();
    app.add_systems(Startup, setup_pending_handoff);
    app.add_systems(Update, route_inbound_messages);
    app.add_systems(Update, complete_handoffs.after(route_inbound_messages));

    app.update();

    let entity = app.world().resource::<TestEntity>().0;
    let mut bus = app.world_mut().resource_mut::<AuthorityBus>();
    bus.inbound.push_back(AuthorityEnvelope::HandoffReject(HandoffReject { entity_id: 100 }));
    drop(bus);

    app.update();

    assert_eq!(app.world().entity(entity).get::<AuthorityState>(), Some(&AuthorityState::Owned));
    assert!(!app.world().entity(entity).contains::<crate::authority::HandoffRequestState>());
}