use crate::{Boundary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};
use crate::ability_type::AbilityType;
use crate::attribute_type::AttributeType;

#[repr(u8)]
pub enum TopicDomain {
    ShardCreated = 0x01,
    PlayerStartingPosition = 0x02,
    PlayerStartingPositionInShard = 0x03,
    Input = 0x04,
    EntityPositionUpdate = 0x05,
    ReleaseOwnership = 0xFD,
    Disconnect = 0xFF,
    ClaimOwnership = 0xFE,
    QuadtreeBoundariesUpdate = 0x06,
    AuthorityDebugPacket = 0x07,

    // Ability & attribute-related topics
    RequestCastAbility = 0xA0,
    CastAbility = 0xA1,
    AbilityHitEntity = 0xA2,
    AttributeUpdated = 0xA3,
    EntityKilled = 0xA4,
    XPEarned = 0xA5,
    LevelUp = 0xA6,
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
    ClaimOwnership(Uuid), // Target shard UUID, payload contains entity UUID
    ReleaseOwnership(Uuid), // Target shard UUID, payload contains entity UUID

    //client topics
    EntityPositionUpdate(Uuid),
    QuadtreeBoundariesUpdate,
    AuthorityDebugPacket(Uuid),

    // Ability & attribute-related topics
    RequestCastAbility,
    CastAbility(Uuid),
    AbilityHitEntity,
    AttributeUpdated(Uuid),
    EntityKilled(Uuid),
    XPEarned(Uuid),
    LevelUp(Uuid),

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
            Topic::EntityPositionUpdate(connection_id) => {
                bytes[0] = TopicDomain::EntityPositionUpdate as u8;
                bytes[1..17].copy_from_slice(connection_id.as_bytes());
            }
            Topic::Disconnect(uuid) => {
                bytes[0] = TopicDomain::Disconnect as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::ReleaseOwnership(uuid) => {
                bytes[0] = TopicDomain::ReleaseOwnership as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::ClaimOwnership(uuid) => {
                bytes[0] = TopicDomain::ClaimOwnership as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::QuadtreeBoundariesUpdate => {
                bytes[0] = TopicDomain::QuadtreeBoundariesUpdate as u8;
            }
            Topic::AuthorityDebugPacket(uuid) => {
                bytes[0] = TopicDomain::AuthorityDebugPacket as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::RequestCastAbility => {
                bytes[0] = TopicDomain::RequestCastAbility as u8;
            }
            Topic::CastAbility(uuid) => {
                bytes[0] = TopicDomain::CastAbility as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::AbilityHitEntity => {
                bytes[0] = TopicDomain::AbilityHitEntity as u8;
            }
            Topic::AttributeUpdated(uuid) => {
                bytes[0] = TopicDomain::AttributeUpdated as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::EntityKilled(uuid) => {
                bytes[0] = TopicDomain::EntityKilled as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::XPEarned(uuid) => {
                bytes[0] = TopicDomain::XPEarned as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::LevelUp(uuid) => {
                bytes[0] = TopicDomain::LevelUp as u8;
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
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::EntityPositionUpdate(uuid)
            }
            0xFF => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::Disconnect(uuid)
            }
            0xFD => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::ReleaseOwnership(uuid)
            }
            0xFE => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::ClaimOwnership(uuid)
            }
            0x06 => Topic::QuadtreeBoundariesUpdate,
            0x07 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::AuthorityDebugPacket(uuid)
            }
            0xA0 => Topic::RequestCastAbility,
            0xA1 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::CastAbility(uuid)
            }
            0xA2 => Topic::AbilityHitEntity,
            0xA3 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::AttributeUpdated(uuid)
            }
            0xA4 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::EntityKilled(uuid)
            }
            0xA5 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::XPEarned(uuid)
            }
            0xA6 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::LevelUp(uuid)
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
pub struct PositionPayload {
    pub position: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct StartingPositionPayload {
    pub connection_id: Uuid,
    pub position: [f64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct QuadtreeBoundariesUpdatePayload {
    pub margin: f32,
    pub boundaries: Vec<Boundary>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct InputPayload {
    pub dxdy: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct AuthorityDebugPacketPayload {
    pub sender_id: Uuid,
}

/// ReleaseOwnership payload (enitity UUID and shard UUID so it can publish claimOwnership )
#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct ReleaseOwnershipPayload {
    pub entity_id: Uuid,
    pub shard_id: Uuid,
}

/// ClaimOwnership payload (enitity UUID and speed so shard can keep the simulation)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct ClaimOwnershipPayload {
    pub entity_id: Uuid,
    pub entity_position: [f64; 2],
    pub speed: [f64; 2],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct UseAbilityPayload {
    pub entity_id: Uuid,
    pub ability: AbilityType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct AttributeUpdatedPayload {
    pub entity_id: Uuid,
    pub attribute: AttributeType,
    pub new_value: i32,
}


#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct AbilityHitEntityPayload {
    pub caster_id: Uuid,
    pub hit_entity_id: Uuid,
    pub ability_type: AbilityType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct EntityKilledPayload {
    pub killer_id: Uuid,
    pub victim_id: Uuid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct XPEarnedPayload {
    pub entity_id: Uuid,
    pub new_value: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, PartialEq)]
pub struct LevelUpPayload {
    pub entity_id: Uuid,
    pub new_level: i32,
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

pub fn serialize_starting_position_payload(payload: &StartingPositionPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize starting position payload")
}

pub fn deserialize_starting_position_payload(bytes: &[u8]) -> Option<StartingPositionPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_quadtree_boundaries_update_payload(payload: &QuadtreeBoundariesUpdatePayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize quadtree boundaries update payload")
}

pub fn deserialize_quadtree_boundaries_update_payload(bytes: &[u8]) -> Option<QuadtreeBoundariesUpdatePayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_authority_debug_packet_payload(payload: &AuthorityDebugPacketPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize authority debug packet payload")
}

pub fn deserialize_authority_debug_packet_payload(bytes: &[u8]) -> Option<AuthorityDebugPacketPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_release_ownership_payload(payload: &ReleaseOwnershipPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize release ownership payload")
}

pub fn deserialize_release_ownership_payload(bytes: &[u8]) -> Option<ReleaseOwnershipPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_claim_ownership_payload(payload: &ClaimOwnershipPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize claim ownership payload")
}

pub fn deserialize_claim_ownership_payload(bytes: &[u8]) -> Option<ClaimOwnershipPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_use_ability_payload(payload: &UseAbilityPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize useability payload")
}

pub fn deserialize_use_ability_payload(bytes: &[u8]) -> Option<UseAbilityPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_attribute_updated_payload(payload: &AttributeUpdatedPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize attribute updated payload")
}

pub fn deserialize_attribute_updated_payload(bytes: &[u8]) -> Option<AttributeUpdatedPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_ability_hit_entity_payload(payload: &AbilityHitEntityPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize ability hit entity payload")
}

pub fn deserialize_ability_hit_entity_payload(bytes: &[u8]) -> Option<AbilityHitEntityPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_entity_killed_payload(payload: &EntityKilledPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize entity killed payload")
}

pub fn deserialize_entity_killed_payload(bytes: &[u8]) -> Option<EntityKilledPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_xp_earned_payload(payload: &XPEarnedPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize XP earned payload")
}

pub fn deserialize_xp_earned_payload(bytes: &[u8]) -> Option<XPEarnedPayload> {
    wincode::deserialize(bytes).ok()
}

pub fn serialize_level_up_payload(payload: &LevelUpPayload) -> Vec<u8> {
    wincode::serialize(payload).expect("failed to serialize level up payload")
}

pub fn deserialize_level_up_payload(bytes: &[u8]) -> Option<LevelUpPayload> {
    wincode::deserialize(bytes).ok()
}