use uuid::Uuid;

#[repr(u8)]
pub enum TopicDomain {
    ShardCreated = 0x01,
    Position = 0x02,
    Input = 0x03,
    ShardSnapshot = 0x04,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    ShardCreated,        // For shard creation events
    Position,   // For specific positions updates
    Input(Uuid), // For client inputs updates, uuid identifies the client
    ShardSnapshot(Uuid), // For shard snapshot updates, uuid identifies the shard
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
            _ => Topic::Raw(bytes),
        }
    }
}
