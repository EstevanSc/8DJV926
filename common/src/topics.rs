use uuid::Uuid;

#[repr(u8)]
pub enum TopicDomain {
    Shard = 0x01,
    Client = 0x02,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    Shard(Uuid),        // Using u32 as per your assignment's shard_id
    Client(Uuid),       // Using u32 as per your assignment's client_id
    Raw([u8; 32]),     // Fallback
}

impl Topic {
    /// Serializes the Topic into a fixed 32-byte array
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        match self {
            Topic::Shard(uuid) => {
                bytes[0] = TopicDomain::Shard as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::Client(uuid) => {
                bytes[0] = TopicDomain::Client as u8;
                bytes[1..17].copy_from_slice(uuid.as_bytes());
            }
            Topic::Raw(raw) => return *raw,
        }
        bytes
    }

    /// Deserializes from a 32-byte array
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        match bytes[0] {
            0x01 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::Shard(uuid)
            }
            0x02 => {
                let uuid = Uuid::from_slice(&bytes[1..17]).unwrap_or_else(|_| Uuid::nil());
                Topic::Client(uuid)
            }
            _ => Topic::Raw(bytes),
        }
    }
}
