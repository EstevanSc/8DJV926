use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead, Eq, PartialEq, Hash)]
pub enum AbilityType {
    Fireball {direction: (i32, i32)},
    Heal,
}