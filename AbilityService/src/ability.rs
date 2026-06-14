use std::time::SystemTime;
use common::ability_type::AbilityType;
use common::attribute_type::AttributeType;
use crate::entity::Entity;

pub struct AttributeEffect {
    pub attribute_type: AttributeType,
    pub modifier_value: i32,    // Value that will be added to the attribute's current value
}

pub struct Ability {
    pub cooldown: f32,
    pub mana_cost: i32,
    pub last_cast: SystemTime,
    pub ability_type: AbilityType,
    pub effects: Vec<AttributeEffect>,
}

impl Ability {
    pub fn from_type(ability_type: AbilityType) -> Ability {
        match ability_type {
            AbilityType::Fireball { direction: _ } => {
                Ability {
                    cooldown: 2f32,
                    mana_cost: 25,
                    last_cast: SystemTime::UNIX_EPOCH,
                    effects: vec![AttributeEffect {
                        attribute_type: AttributeType::HealthPoints,
                        modifier_value: -20}],
                    ability_type,
                }
            }
            AbilityType::Heal => {
                Ability {
                    cooldown: 5f32,
                    mana_cost: 35,
                    last_cast: SystemTime::UNIX_EPOCH,
                    effects: vec![AttributeEffect {
                        attribute_type: AttributeType::HealthPoints,
                        modifier_value: 50}],
                    ability_type,
                }
            }
        }
    }
    pub fn can_be_casted(&self, entity: &Entity) -> bool {
        // Entity has enough mana
        if let Some(mana_attribute) = entity.attributes.get(&AttributeType::ManaPoints) {
            if mana_attribute.current_value < self.mana_cost {
                return false;
            }
        }
        // Entity has ability
        if let Some(ability) = entity.abilities.get(&self.ability_type) {
            // Cooldown has been elapsed
            let elapsed_time = SystemTime::now().duration_since(ability.last_cast).unwrap();
            if elapsed_time.as_secs_f32() >= self.cooldown {
                return true;
            }
        }

        false
    }

    pub fn update_last_cast(&mut self) {
        self.last_cast = SystemTime::now();
    }
}