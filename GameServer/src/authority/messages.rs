use bevy::prelude::Vec2;
use bytes::{Buf, BufMut, Bytes, BytesMut};

const TAG_HANDOFF_REQUEST: u8 = 0x20;
const TAG_HANDOFF_ACCEPT: u8 = 0x21;
const TAG_HANDOFF_REJECT: u8 = 0x22;
const TAG_GHOST_UPDATE: u8 = 0x23;
const TAG_HANDOFF_COMPLETE: u8 = 0x24;

const TAG_SIZE: usize = 1;
const U32_SIZE: usize = 4;
const VEC2_SIZE: usize = 4 * 2;
const STATE_SIZE: usize = 64;

const HANDOFF_REQUEST_SIZE: usize =
    TAG_SIZE + U32_SIZE + VEC2_SIZE + VEC2_SIZE + STATE_SIZE;

/// Handoff request sent from one shard to another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandoffRequest {
    pub entity_id: u32,
    pub pos: Vec2,
    pub vel: Vec2,
    pub state: [u8; 64],
}

/// Positive handoff reply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandoffAccept {
    pub entity_id: u32,
}

/// Negative handoff reply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandoffReject {
    pub entity_id: u32,
}

/// Lightweight position update for a ghost entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GhostUpdate {
    pub entity_id: u32,
    pub pos: Vec2,
}

/// Final handoff signal once authority is transferred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandoffComplete {
    pub entity_id: u32,
}

/// Typed envelope for all authority messages.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthorityEnvelope {
    HandoffRequest(HandoffRequest),
    HandoffAccept(HandoffAccept),
    HandoffReject(HandoffReject),
    GhostUpdate(GhostUpdate),
    HandoffComplete(HandoffComplete),
}

/// Codec errors for authority packets.
#[derive(Debug)]
pub enum AuthorityCodecError {
    EmptyPacket,
    InvalidTag(u8),
    Truncated,
    TrailingBytes,
}

impl AuthorityEnvelope {
    /// Encodes the message to the wire format.
    pub fn encode(&self) -> Bytes {
        let mut bytes = BytesMut::with_capacity(HANDOFF_REQUEST_SIZE);

        match self {
            Self::HandoffRequest(message) => {
                bytes.put_u8(TAG_HANDOFF_REQUEST);
                bytes.put_u32_le(message.entity_id);
                bytes.put_f32_le(message.pos.x);
                bytes.put_f32_le(message.pos.y);
                bytes.put_f32_le(message.vel.x);
                bytes.put_f32_le(message.vel.y);
                bytes.extend_from_slice(&message.state);
            }
            Self::HandoffAccept(message) => {
                bytes.put_u8(TAG_HANDOFF_ACCEPT);
                bytes.put_u32_le(message.entity_id);
            }
            Self::HandoffReject(message) => {
                bytes.put_u8(TAG_HANDOFF_REJECT);
                bytes.put_u32_le(message.entity_id);
            }
            Self::GhostUpdate(message) => {
                bytes.put_u8(TAG_GHOST_UPDATE);
                bytes.put_u32_le(message.entity_id);
                bytes.put_f32_le(message.pos.x);
                bytes.put_f32_le(message.pos.y);
            }
            Self::HandoffComplete(message) => {
                bytes.put_u8(TAG_HANDOFF_COMPLETE);
                bytes.put_u32_le(message.entity_id);
            }
        }

        bytes.freeze()
    }

    /// Decodes a wire packet into a typed message.
    pub fn decode(raw: &[u8]) -> Result<Self, AuthorityCodecError> {
        let mut cursor = raw;
        if cursor.is_empty() {
            return Err(AuthorityCodecError::EmptyPacket);
        }

        let tag = cursor.get_u8();
        let message = match tag {
            TAG_HANDOFF_REQUEST => {
                let entity_id = read_u32(&mut cursor)?;
                let pos = read_vec2(&mut cursor)?;
                let vel = read_vec2(&mut cursor)?;
                let state = read_fixed_64(&mut cursor)?;
                Self::HandoffRequest(HandoffRequest { entity_id, pos, vel, state })
            }
            TAG_HANDOFF_ACCEPT => Self::HandoffAccept(HandoffAccept {
                entity_id: read_u32(&mut cursor)?,
            }),
            TAG_HANDOFF_REJECT => Self::HandoffReject(HandoffReject {
                entity_id: read_u32(&mut cursor)?,
            }),
            TAG_GHOST_UPDATE => {
                let entity_id = read_u32(&mut cursor)?;
                let pos = read_vec2(&mut cursor)?;
                Self::GhostUpdate(GhostUpdate { entity_id, pos })
            }
            TAG_HANDOFF_COMPLETE => Self::HandoffComplete(HandoffComplete {
                entity_id: read_u32(&mut cursor)?,
            }),
            other => return Err(AuthorityCodecError::InvalidTag(other)),
        };

        if !cursor.is_empty() {
            return Err(AuthorityCodecError::TrailingBytes);
        }

        Ok(message)
    }
}

/// Reads a little-endian u32 from the cursor.
fn read_u32(cursor: &mut &[u8]) -> Result<u32, AuthorityCodecError> {
    if cursor.len() < 4 {
        return Err(AuthorityCodecError::Truncated);
    }

    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&cursor[..4]);
    *cursor = &cursor[4..];
    Ok(u32::from_le_bytes(bytes))
}

/// Reads a little-endian Vec2 from the cursor.
fn read_vec2(cursor: &mut &[u8]) -> Result<Vec2, AuthorityCodecError> {
    let x = read_f32(cursor)?;
    let y = read_f32(cursor)?;
    Ok(Vec2::new(x, y))
}

/// Reads a little-endian f32 from the cursor.
fn read_f32(cursor: &mut &[u8]) -> Result<f32, AuthorityCodecError> {
    if cursor.len() < 4 {
        return Err(AuthorityCodecError::Truncated);
    }

    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&cursor[..4]);
    *cursor = &cursor[4..];
    Ok(f32::from_le_bytes(bytes))
}

/// Reads the fixed 64-byte authority state payload.
fn read_fixed_64(cursor: &mut &[u8]) -> Result<[u8; 64], AuthorityCodecError> {
    if cursor.len() < 64 {
        return Err(AuthorityCodecError::Truncated);
    }

    let mut payload = [0u8; 64];
    payload.copy_from_slice(&cursor[..64]);
    *cursor = &cursor[64..];
    Ok(payload)
}