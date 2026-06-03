use crate::{Boundary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

#[repr(u8)]
pub enum TopicDomain {
    ShardCreated = 0x01,
    PlayerStartingPosition = 0x02,
    PlayerStartingPositionInShard = 0x03,
    Input = 0x04,
    EntityPositionUpdate = 0x05,
    Disconnect = 0xFF,
    ClaimOwnership = 0xFE,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    //Quadtree topics
    ShardCreated,       
    PlayerStartingPosition, 

    //server topics
    PlayerStartingPositionInShard(Uuid), 
    Input(Uuid), // For client inputs updates, uuid identifies the client
    Disconnect(Uuid), // For disconnect events
    ClaimOwnership(Uuid), // For claiming ownership of an entity, uuid the shard

    //client topics
    EntityPositionUpdate(u32), 

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
            Topic::PlayerStartingPosition => {
                bytes[0] = TopicDomain::PlayerStartingPosition as u8;
            }
            Topic::PlayerStartingPositionInShard(uuid) => {
                bytes[0] = TopicDomain::PlayerStartingPositionInShard as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::Input(uuid) => {
                bytes[0] = TopicDomain::Input as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::EntityPositionUpdate(entity_id) => {
                bytes[0] = TopicDomain::EntityPositionUpdate as u8;
                bytes[1..5].copy_from_slice(&entity_id.to_be_bytes());
            }
            Topic::Disconnect(uuid) => {
                bytes[0] = TopicDomain::Disconnect as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::ClaimOwnership(uuid) => {
                bytes[0] = TopicDomain::ClaimOwnership as u8;
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
                Topic::PlayerStartingPosition
            }
            0x03 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::PlayerStartingPositionInShard(uuid)
            }
            0x04 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::Input(uuid)
            }
            0x05 => {
                let entity_id = u32::from_be_bytes(bytes[1..5].try_into().unwrap_or_default());
                Topic::EntityPositionUpdate(entity_id)
            }
            0xFF => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::Disconnect(uuid)
            }
            0xFE => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::ClaimOwnership(uuid)
             }
            _ => Topic::Raw(bytes),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct ShardCreatedPayload {
    pub shard_connection_id: Uuid,
    pub boundary: Boundary, // to fix
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct PlayerStartingPositionPayload {
    pub player_id: Uuid,
    pub position: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct PositionPayload {
    pub entity_id: u32,
    pub position: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct InputPayload {
    pub player_id: Uuid,
    pub dxdy: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct DisconnectPayload {
    pub player_id: Uuid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct ClaimOwnershipPayload {
    pub entity_id: u32,
}

pub fn serialize_shard_created_payload(payload: &ShardCreatedPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize shard created payload")
}

pub fn deserialize_shard_created_payload(bytes: &[u8]) -> Option<ShardCreatedPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_player_starting_position_payload(payload: &PlayerStartingPositionPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize player starting position payload")
}

pub fn deserialize_player_starting_position_payload(bytes: &[u8]) -> Option<PlayerStartingPositionPayload> {
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

pub fn serialize_disconnect_payload(payload: &DisconnectPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize disconnect payload")
}

pub fn deserialize_disconnect_payload(bytes: &[u8]) -> Option<DisconnectPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_claim_ownership_payload(payload: &ClaimOwnershipPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize claim ownership payload")
}

pub fn deserialize_claim_ownership_payload(bytes: &[u8]) -> Option<ClaimOwnershipPayload> {
    wincode::deserialize(bytes).ok()
}
