use bevy::prelude::Vec2;

use crate::authority::{
    AuthorityEnvelope, GhostUpdate, HandoffAccept, HandoffComplete, HandoffReject, HandoffRequest,
};

/// Provides encode/decode methods for authority messages.
fn sample_state() -> [u8; 64] {
    let mut state = [0u8; 64];
    for (index, byte) in state.iter_mut().enumerate() {
        *byte = index as u8;
    }
    state
}

/// Verifies the handoff request codec roundtrips.
#[test]
fn handoff_request_roundtrips() {
    let envelope = AuthorityEnvelope::HandoffRequest(HandoffRequest {
        entity_id: 7,
        pos: Vec2::new(1.5, -2.5),
        vel: Vec2::new(3.25, 4.75),
        state: sample_state(),
    });

    let encoded = envelope.encode();
    assert_eq!(encoded.len(), 1 + 4 + 8 + 8 + 64);

    let decoded = AuthorityEnvelope::decode(&encoded).expect("request should decode");
    assert_eq!(decoded, envelope);
}

/// Verifies the non-request authority messages roundtrip.
#[test]
fn authority_messages_roundtrip() {
    let messages = [
        AuthorityEnvelope::HandoffAccept(HandoffAccept { entity_id: 11 }),
        AuthorityEnvelope::HandoffReject(HandoffReject { entity_id: 12 }),
        AuthorityEnvelope::GhostUpdate(GhostUpdate {
            entity_id: 13,
            pos: Vec2::new(-10.0, 20.0),
        }),
        AuthorityEnvelope::HandoffComplete(HandoffComplete { entity_id: 14 }),
    ];

    for envelope in messages {
        let encoded = envelope.encode();
        let decoded = AuthorityEnvelope::decode(&encoded).expect("message should decode");
        assert_eq!(decoded, envelope);
    }
}

/// Verifies invalid tags are rejected.
#[test]
fn decode_rejects_invalid_tag() {
    let decoded = AuthorityEnvelope::decode(&[0x99]);
    assert!(decoded.is_err());
}