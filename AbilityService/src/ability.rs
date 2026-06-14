use std::time::SystemTime;
use crate::attribute::AttributeType;
use crate::entity::Entity;

#[derive(Eq, PartialEq)]
pub enum AbilityType {
    Fireball {direction: (i32, i32)},
    Heal,
}

pub struct Ability {
    pub cooldown: f32,
    pub mana_cost: f32,
    pub last_cast: SystemTime,
    pub ability_type: AbilityType,
}

impl Ability {
    pub fn can_be_casted(&self, entity: Entity) -> bool {
        // Entity has enough mana
        if let Some(mana_attribute) = entity.attributes.iter().find(|attribute| attribute.attribute_type == AttributeType::ManaPoints) {
            if mana_attribute.current_value < self.mana_cost {
                return false;
            }
        }
        // Entity has ability
        if let Some(ability) = entity.abilities.iter().find(|ability| ability.ability_type == self.ability_type) {
            // Cooldown has been elapsed
            let elapsed_time = SystemTime::now().duration_since(ability.last_cast).unwrap();
            if elapsed_time.as_secs_f32() >= self.cooldown {
                return true;
            }
        }

        false
    }
}