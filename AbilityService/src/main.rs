use std::collections::HashMap;
use std::time::Duration;
use common::broker_api::BrokerClient;
use common::broker_messages::SendingSystem;
use common::topics::{deserialize_starting_position_payload, deserialize_use_ability_payload, serialize_use_ability_payload, Topic, UseAbilityPayload};
use crate::attribute::AttributeType;
use crate::config::Config;
use crate::entity::Entity;

mod entity;
mod attribute;
mod ability;
mod config;

#[tokio::main]
async fn main()
{
    let config = Config::from_env();
    let host = config.broker_host.clone();
    let port = config.broker_port;
    if let Ok(mut client) = BrokerClient::connect(host.as_str(), port, SendingSystem::AbilityService).await
    {
        // Client connection
        if let Err(e) = client.subscribe(Topic::PlayerStartingPosition).await {
            tracing::error!("Failed to subscribe to ability service: {:?}", e);
        }
        if let Err(e) = client.subscribe(Topic::RequestCastAbility).await {
            tracing::error!("Failed to subscribe to ability service: {:?}", e);
        }
        if let Err(e) = client.subscribe(Topic::AbilityHitEntity).await {
            tracing::error!("Failed to subscribe to ability service: {:?}", e);
        }

        run_main_loop(&config, &mut client).await;
    }

}

async fn run_main_loop(config: &Config, client: &mut BrokerClient) {
    let mut tick = tokio::time::interval(Duration::from_millis(config.ability_service_tick_ms));
    let mut entity_registry: HashMap<uuid::Uuid, Entity> = HashMap::new();
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
                                        return;
                                    };

                                    // Send cast_ability message
                                    let cast_topic = Topic::CastAbility(entity_id);
                                    let cast_payload = UseAbilityPayload {entity_id, ability: ability_type};
                                    let raw_payload = serialize_use_ability_payload(&cast_payload);
                                    if let Err(e) = client.publish_raw(cast_topic, raw_payload.as_slice()).await {
                                        tracing::error!("Failed to publish raw ability payload: {:?}", e);
                                    }

                                    // Update entity's mana
                                    if let Some(mana_attribute) = entity.attributes.get(&AttributeType::ManaPoints) {
                                        let new_mana = mana_attribute.current_value - mana_cost;
                                        entity.update_attribute(AttributeType::ManaPoints, new_mana);
                                    }

                                }
                            }
                        }
                    }
                    Topic::AbilityHitEntity => {}
                    _ => {}
                }
            }
        }

        tick.tick().await;
    }
}