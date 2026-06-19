use std::collections::HashMap;
use std::time::Duration;
use anyhow::Context;
use common::broker_api::BrokerClient;
use common::broker_messages::SendingSystem;
use common::topics::{deserialize_ability_hit_entity_payload, deserialize_starting_position_payload, deserialize_use_ability_payload, serialize_attribute_updated_payload, serialize_entity_killed_payload, serialize_level_up_payload, serialize_use_ability_payload, serialize_xp_earned_payload, AttributeUpdatedPayload, EntityKilledPayload, LevelUpPayload, Topic, UseAbilityPayload, XPEarnedPayload};
use common::attribute_type::AttributeType;
use crate::ability::Ability;
use crate::config::Config;
use crate::entity::Entity;

mod entity;
mod attribute;
mod ability;
mod config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();
    let host = config.broker_host.clone();
    let port = config.broker_port;

    let mut client = match BrokerClient::connect(host.as_str(), port, SendingSystem::AbilityService).await {
        Ok(client) => {
            tracing::info!("Ability Service successfully connected to broker");
            client
        }
        Err(e) => {
            return Err(anyhow::anyhow!(e).context(format!("Failed to connect to broker at {}:{}", host, port)));
        }
    };

    // 2. Subscribe to topics
    client.subscribe(Topic::PlayerStartingPosition).await
        .context("Failed to subscribe to PlayerStartingPosition")?;

    client.subscribe(Topic::RequestCastAbility).await
        .context("Failed to subscribe to RequestCastAbility")?;

    client.subscribe(Topic::AbilityHitEntity).await
        .context("Failed to subscribe to AbilityHitEntity")?;

    run_main_loop(&config, &mut client).await;

    Ok(())
}

async fn run_main_loop(config: &Config, client: &mut BrokerClient) {
    let mut tick = tokio::time::interval(Duration::from_millis(config.ability_service_tick_ms));
    let mut entity_registry: HashMap<uuid::Uuid, Entity> = HashMap::new();

    let mut mana_regen_accumulator = 0.0f32; 
    loop {
        if let Ok(messages) =client.poll_broadcasts() {
            for (topic, payload) in messages {
                match topic {
                    Topic::PlayerStartingPosition => {
                        if let Some(position_payload) = deserialize_starting_position_payload(&payload)
                        {
                            let entity_id = position_payload.connection_id;
                            let new_entity = Entity::default(entity_id);
                            entity_registry.entry(entity_id).or_insert(new_entity);
                        }
                    }

                    Topic::RequestCastAbility => {
                        if let Some(ability_payload) = deserialize_use_ability_payload(&payload) {
                            let entity_id = ability_payload.entity_id;
                            if let Some(entity) = entity_registry.get_mut(&entity_id) {
                                if entity.can_cast_ability(ability_payload.ability) {
                                    let (ability_type, mana_cost) = if let Some(mut_ability) = entity.find_ability_mut(ability_payload.ability) {
                                        mut_ability.update_last_cast();
                                        // Copy/Clone the small data pieces we need later
                                        (mut_ability.ability_type, mut_ability.mana_cost)
                                    } else {
                                        tracing::warn!("Entity {} tried to cast unregistered ability: {:?}", entity_id, ability_payload.ability);
                                        continue;
                                    };

                                    // Send cast_ability message
                                    let cast_topic = Topic::CastAbility(entity_id);
                                    let cast_payload = UseAbilityPayload {entity_id, ability: ability_type, direction:ability_payload.direction};
                                    let raw_payload = serialize_use_ability_payload(&cast_payload);
                                    if let Err(e) = client.publish_raw(cast_topic, raw_payload.as_slice()).await {
                                        tracing::error!("Failed to publish raw ability payload: {:?}", e);
                                    }

                                    // Update entity's mana
                                    if let Some(mana_attribute) = entity.attributes.get(&AttributeType::ManaPoints) {
                                        let new_mana = mana_attribute.current_value - mana_cost;
                                        entity.update_attribute(AttributeType::ManaPoints, new_mana);

                                        // Send attribute update to entities
                                        let attribute_updated_payload = AttributeUpdatedPayload {
                                            entity_id,
                                            attribute:AttributeType::ManaPoints,
                                            new_value: new_mana,
                                        };
                                        let raw_payload = serialize_attribute_updated_payload(&attribute_updated_payload);
                                        tracing::info!("Mana updated for entity {}: {}", entity_id, new_mana);
                                        if let Err(e) = client.publish_raw(Topic::AttributeUpdated(entity_id), raw_payload.as_slice()).await {
                                            tracing::error!("Failed to publish attribute updated payload: {:?}", e);
                                        }
                                    }

                                }
                            }
                        }
                    }

                    Topic::AbilityHitEntity => {
                        if let Some(ability_payload) = deserialize_ability_hit_entity_payload(&payload) {
                            let hit_entity_id = ability_payload.hit_entity_id;

                            tracing::info!("AbilityHitEntity received for entity {:?}", hit_entity_id);
                            let mut entity_was_killed = false;
                            let caster_level = entity_registry.get(&ability_payload.caster_id).map(|e| e.level).unwrap_or(1);
                            if let Some(hit_entity) = entity_registry.get_mut(&hit_entity_id) {
                                let ability = Ability::from_type(ability_payload.ability_type);
                                for effect in ability.effects {
                                    if let Some(attribute) = hit_entity.attributes.get(&effect.attribute_type) {
                                        let mut multiplier = 1.0; // Base multiplier, can be adjusted based on various factors
                                        multiplier = multiplier + (caster_level as f32 * 0.5);
                                        let new_value = attribute.current_value + (effect.modifier_value as f32 * multiplier) as i32;
                                        if let Some(updated_value) =hit_entity.update_attribute(effect.attribute_type, new_value) {
                                            // Send entity attribute update
                                            let attribute_updated_payload = AttributeUpdatedPayload {
                                                entity_id: hit_entity_id,
                                                attribute:effect.attribute_type,
                                                new_value: updated_value,
                                            };
                                            tracing::info!("Attribute updated for entity {}: {:?} = {}", hit_entity_id, effect.attribute_type, updated_value);

                                            let raw_payload = serialize_attribute_updated_payload(&attribute_updated_payload);
                                            if let Err(e) = client.publish_raw(Topic::AttributeUpdated(hit_entity_id), raw_payload.as_slice()).await {
                                                tracing::error!("Failed to publish attribute updated payload: {:?}", e);
                                            }

                                            // Check if entity has been killed
                                            entity_was_killed = effect.attribute_type == AttributeType::HealthPoints && updated_value <= 0;
                                            if entity_was_killed {
                                                // Send entity killed message
                                                let entity_killed_payload: EntityKilledPayload = EntityKilledPayload {
                                                    killer_id: ability_payload.caster_id,
                                                    victim_id: ability_payload.hit_entity_id,
                                                };
                                                let raw_payload = serialize_entity_killed_payload(&entity_killed_payload);
                                                if let Err(e) = client.publish_raw(Topic::EntityKilled(hit_entity_id), raw_payload.as_slice()).await {
                                                    tracing::error!("Failed to publish entity killed payload: {:?}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            else {
                                tracing::error!("Couldn't find entity: {:?}", hit_entity_id);
                            }

                            if entity_was_killed {
                                // Update killed xp
                                let caster_id = ability_payload.caster_id;
                                if let Some(caster) = entity_registry.get_mut(&caster_id) {
                                    caster.experience_points += 25;
                                    while caster.experience_points >= 100 {
                                        // Level up
                                        caster.level += 1;
                                        caster.experience_points -= 100;

                                        // Send level up message
                                        let lvl_up_payload = LevelUpPayload {
                                            entity_id: caster_id,
                                            new_level: caster.level,
                                        };
                                        let raw_payload = serialize_level_up_payload(&lvl_up_payload);
                                        if let Err(e) = client.publish_raw(Topic::LevelUp(caster_id), raw_payload.as_slice()).await {
                                            tracing::error!("Failed to publish entity level up payload: {:?}", e);
                                        }
                                    }

                                    // Send xp update message
                                    let xp_update_payload = XPEarnedPayload {
                                        entity_id: caster_id,
                                        new_value: caster.experience_points,
                                    };
                                    let raw_payload = serialize_xp_earned_payload(&xp_update_payload);
                                    if let Err(e) = client.publish_raw(Topic::XPEarned(caster_id), raw_payload.as_slice()).await {
                                        tracing::error!("Failed to publish entity xp update payload: {:?}", e);
                                    }
                                }
                            }
                        }
                        else {
                            tracing::error!("Failed deserialize ability hit entity payload: {:?}", payload);
                        }
                    }
                    _ => {}
                }
            }
        }

        // mana regeneration each second
        mana_regen_accumulator += config.ability_service_tick_ms as f32 / 1000.0;
        
        if mana_regen_accumulator >= 1.0 {
            for (entity_id, entity) in entity_registry.iter_mut() {
                if let Some(mana_attr) = entity.attributes.get_mut(&AttributeType::ManaPoints) {
                    if mana_attr.current_value < mana_attr.max_value {
                        let new_mana = (mana_attr.current_value + 5).min(mana_attr.max_value);
                        entity.update_attribute(AttributeType::ManaPoints, new_mana);

                        let payload = serialize_attribute_updated_payload(&AttributeUpdatedPayload {
                            entity_id: *entity_id,
                            attribute: AttributeType::ManaPoints,
                            new_value: new_mana,
                        });
                        let _ = client.publish_raw(Topic::AttributeUpdated(*entity_id), payload.as_slice()).await;
                    }
                }
            }
            mana_regen_accumulator = 0.0;
        }

        tick.tick().await;
    }
}