use crate::Vec2;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

#[repr(u8)]
pub enum TopicDomain {
    ShardCreated = 0x01,
    Position = 0x02,
    Input = 0x03,
    ShardSnapshot = 0x04,
    ForcedPositionUpdate = 0x05,
    CrossingAlert = 0x10,
    HandoffRequest = 0x20,
    HandoffAccept = 0x21,
    HandoffReject = 0x22,
    GhostUpdate = 0x23,
    HandoffComplete = 0x24,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    ShardCreated,        // For shard creation events
    Position,   // For specific positions updates
    Input(Uuid), // For client inputs updates, uuid identifies the client
    ForcedPositionUpdate(Uuid), // For forced position updates, uuid identifies the entity
    ShardSnapshot(Uuid), // For shard snapshot updates, uuid identifies the shard
    CrossingAlert(Uuid),   // For quadtree to alert a shard, uuid identifies the source shard
    HandoffRequest(Uuid),  // uuid identifies the destination shard
    HandoffAccept(Uuid),   // uuid identifies the source shard
    HandoffReject(Uuid),   // uuid identifies the source shard
    GhostUpdate(Uuid),     // uuid identifies the destination shard
    HandoffComplete(Uuid), // uuid identifies the source shard
    Raw([u8; 32]),     // Fallback
}

impl Topic {
    /// Serializes the Topic into a fixed 32-byte array
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        match self {
            Topic::ShardCreated => {
                bytes[0] = TopicDomain::ShardCreated as u8;
            }
            Topic::Position => {
                bytes[0] = TopicDomain::Position as u8;
            }
            Topic::Input(uuid) => {
                bytes[0] = TopicDomain::Input as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::ShardSnapshot(uuid) => {
                bytes[0] = TopicDomain::ShardSnapshot as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::ForcedPositionUpdate(uuid) => {
                bytes[0] = TopicDomain::ForcedPositionUpdate as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::CrossingAlert(uuid) => {
                bytes[0] = TopicDomain::CrossingAlert as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::HandoffRequest(uuid) => {
                bytes[0] = TopicDomain::HandoffRequest as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::HandoffAccept(uuid) => {
                bytes[0] = TopicDomain::HandoffAccept as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::HandoffReject(uuid) => {
                bytes[0] = TopicDomain::HandoffReject as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::GhostUpdate(uuid) => {
                bytes[0] = TopicDomain::GhostUpdate as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::HandoffComplete(uuid) => {
                bytes[0] = TopicDomain::HandoffComplete as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::Raw(raw) => return *raw,
        }
        bytes
    }

    /// Deserializes from a 32-byte array
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        match bytes[0] {
            0x01 => Topic::ShardCreated,
            0x02 => {
                Topic::Position
            }
            0x03 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::Input(uuid)
            }
            0x04 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::ShardSnapshot(uuid)
            }
            0x05 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::ForcedPositionUpdate(uuid)
            }
            0x10 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::CrossingAlert(uuid)
            }
            0x20 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::HandoffRequest(uuid)
            }
            0x21 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::HandoffAccept(uuid)
            }
            0x22 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::HandoffReject(uuid)
            }
            0x23 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::GhostUpdate(uuid)
            }
            0x24 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::HandoffComplete(uuid)
            }
            _ => Topic::Raw(bytes),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct ShardCreatedPayload {
    pub shard_id: Uuid,
    pub center: Vec2,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct PositionPayload {
    pub entity_id: Uuid,
    pub position: Vec2,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct InputPayload {
    pub player_id: Uuid,
    pub dxdy: Vec2,
}

#[derive(Debug, Clone, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct ShardSnapshotPayload {
    pub shard_id: Uuid,
    pub replication: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct CrossingAlertPayload {
    pub entity_id: u32,
    pub target_shard_id: u32,
    pub target_shard_uuid: Uuid,
}

pub fn serialize_shard_created_payload(payload: &ShardCreatedPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize shard created payload")
}

pub fn deserialize_shard_created_payload(bytes: &[u8]) -> Option<ShardCreatedPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_position_payload(payload: &PositionPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize position payload")
}

pub fn deserialize_position_payload(bytes: &[u8]) -> Option<PositionPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_input_payload(payload: &InputPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize input payload")
}

pub fn deserialize_input_payload(bytes: &[u8]) -> Option<InputPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_shard_snapshot_payload(payload: &ShardSnapshotPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize shard snapshot payload")
}

pub fn deserialize_shard_snapshot_payload(bytes: &[u8]) -> Option<ShardSnapshotPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_crossing_alert_payload(payload: &CrossingAlertPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize crossing alert payload")
}

pub fn deserialize_crossing_alert_payload(bytes: &[u8]) -> Option<CrossingAlertPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_forced_position_update_payload(payload: &PositionPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize forced position update payload")
}

pub fn deserialize_forced_position_update_payload(bytes: &[u8]) -> Option<PositionPayload> {
    wincode::deserialize(bytes).ok()
}