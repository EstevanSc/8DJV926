use std::collections::HashSet;
use crate::ability::Ability;
use crate::attribute::Attribute;

pub struct Entity {
    pub entity_id: uuid::Uuid,
    pub experience_points: f32,
    pub level: i32,
    pub attributes: HashSet<Attribute>,
    pub abilities: HashSet<Ability>,
}