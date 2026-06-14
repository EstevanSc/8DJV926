use std::time::Duration;
use common::broker_api::BrokerClient;
use common::broker_messages::SendingSystem;
use common::topics::Topic;
use crate::config::Config;

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
    loop {
        if let Ok(messages) =client.poll_broadcasts() {
            for _message in messages {
                // Handle messages
            }
        }

        tick.tick().await;
    }
}