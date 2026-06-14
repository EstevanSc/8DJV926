use std::collections::HashMap;
use common::ability_type::AbilityType;
use crate::ability::Ability;
use crate::attribute::{Attribute, AttributeType};

pub struct Entity {
    pub entity_id: uuid::Uuid,
    pub experience_points: f32,
    pub level: i32,
    pub attributes: HashMap<AttributeType, Attribute>,
    pub abilities: HashMap<AbilityType, Ability>,
}

impl Entity {
    pub fn default(entity_id: uuid::Uuid) -> Self {
        let mut attributes = HashMap::new();
        attributes.insert(
            AttributeType::ManaPoints,
            Attribute {
            attribute_type: AttributeType::ManaPoints,
            max_value: 100,
            min_value: 0,
            current_value: 100,
        });
        Self {
            entity_id,
            experience_points: 0f32,
            level: 0,
            attributes,
            abilities: HashMap::new()
        }
    }

    pub fn find_ability(&self, ability: AbilityType) -> Option<&Ability> {
        self.abilities.get(&ability)
    }
    pub fn find_ability_mut(&mut self, ability: AbilityType) -> Option<&mut Ability> {
        self.abilities.get_mut(&ability)
    }

    pub fn can_cast_ability(&self, ability: AbilityType) -> bool {
        if let Some(ability) = self.find_ability(ability) {
            return ability.can_be_casted(self);
        }
        false
    }

    pub fn update_attribute(&mut self, attribute_type: AttributeType, value: i32) {
        if let Some(attribute) = self.attributes.get_mut(&attribute_type) {
            if (value >= attribute.min_value && value <= attribute.max_value){
                attribute.current_value = value;
            }
        }
    }
}