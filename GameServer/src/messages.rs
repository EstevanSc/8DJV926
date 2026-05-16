use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
pub enum GameMessage {
    Join { username: String },
    Welcome { player_id: Uuid },
}