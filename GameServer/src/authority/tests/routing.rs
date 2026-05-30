use std::collections::VecDeque;

use bevy::prelude::*;

use crate::authority::systems::{route_inbound_messages, AuthorityBus};
use crate::authority::{
    AuthorityEnvelope, GhostUpdate, HandoffAccept, HandoffComplete, HandoffReject,
    HandoffRequest,
};

/// Verifies inbound messages are routed into the correct queues.
fn sample_request(entity_id: u32) -> HandoffRequest {
    HandoffRequest {
        entity_id,
        pos: Vec2::new(1.0, 2.0),
        vel: Vec2::new(3.0, 4.0),
        state: [entity_id as u8; 64],
    }
}

/// Verifies inbound messages are routed into the correct queues.
#[test]
fn authority_bus_routes_messages_into_queues() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(AuthorityBus {
        inbound: VecDeque::from([
            AuthorityEnvelope::HandoffRequest(sample_request(1)),
            AuthorityEnvelope::HandoffAccept(HandoffAccept { entity_id: 2 }),
            AuthorityEnvelope::HandoffReject(HandoffReject { entity_id: 3 }),
            AuthorityEnvelope::GhostUpdate(GhostUpdate {
                entity_id: 4,
                pos: Vec2::new(5.0, 6.0),
            }),
            AuthorityEnvelope::HandoffComplete(HandoffComplete { entity_id: 5 }),
        ]),
        ..Default::default()
    });

    app.add_systems(Update, route_inbound_messages);
    app.update();

    let bus = app.world().resource::<AuthorityBus>();
    assert!(bus.inbound.is_empty());
    assert_eq!(bus.pending_requests.len(), 1);
    assert_eq!(bus.pending_accepts.len(), 1);
    assert_eq!(bus.pending_rejects.len(), 1);
    assert_eq!(bus.pending_ghost_updates.len(), 1);
    assert_eq!(bus.pending_completes.len(), 1);

    assert_eq!(bus.pending_requests.front().map(|message| message.entity_id), Some(1));
    assert_eq!(bus.pending_accepts.front().map(|message| message.entity_id), Some(2));
    assert_eq!(bus.pending_rejects.front().map(|message| message.entity_id), Some(3));
    assert_eq!(bus.pending_ghost_updates.front().map(|message| message.entity_id), Some(4));
    assert_eq!(bus.pending_completes.front().map(|message| message.entity_id), Some(5));
}