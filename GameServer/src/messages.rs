use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
pub enum GameMessage {
    Join { username: String },
    Welcome { player_id: Uuid },
}
