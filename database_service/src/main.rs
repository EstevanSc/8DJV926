use anyhow::Context;
use common::broker_api::BrokerClient;
use common::broker_messages::SendingSystem;
use common::supabase::SupabaseClient;
use common::topics::{
    DbNameResponsePayload, Topic, deserialize_db_name_request_payload,
    deserialize_db_query_payload, deserialize_db_register_username_payload,
    serialize_db_name_response_payload,
};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

mod config;
use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();
    let host = config.broker_host.clone();
    let port = config.broker_port;

    tracing::info!(
        "Connecting database client to Supabase at {}...",
        config.supabase_url
    );
    let supabase = SupabaseClient::new(&config.supabase_url, &config.supabase_key);

    let client =
        match BrokerClient::connect(host.as_str(), port, SendingSystem::DatabaseService).await {
            Ok(client) => {
                tracing::info!("Database Service successfully connected to broker");
                client
            }
            Err(e) => {
                return Err(anyhow::anyhow!(e)
                    .context(format!("Failed to connect to broker at {}:{}", host, port)));
            }
        };

    client
        .subscribe(Topic::DbQuery)
        .await
        .context("Failed to subscribe to Topic::DbQuery")?;

    client
        .subscribe(Topic::DbRegisterUsername)
        .await
        .context("Failed to subscribe to Topic::DbRegisterUsername")?;

    client
        .subscribe(Topic::DbNameRequest)
        .await
        .context("Failed to subscribe to Topic::DbNameRequest")?;

    tracing::info!("Database Service listening for registrations, updates, and name requests...");

    run_main_loop(supabase, client).await;

    Ok(())
}

async fn run_main_loop(supabase: SupabaseClient, mut client: BrokerClient) {
    let mut tick = tokio::time::interval(Duration::from_millis(5));
    let mut player_map: HashMap<Uuid, String> = HashMap::new();

    loop {
        if let Ok(messages) = client.poll_broadcasts() {
            for (topic, payload) in messages {
                match topic {
                    Topic::DbRegisterUsername => {
                        if let Some(reg_payload) =
                            deserialize_db_register_username_payload(&payload)
                        {
                            tracing::info!(
                                "Registering player correlation: id={}, username={}",
                                reg_payload.player_id,
                                reg_payload.username
                            );
                            player_map.insert(reg_payload.player_id, reg_payload.username);
                        } else {
                            tracing::error!("Failed to deserialize DbRegisterUsernamePayload");
                        }
                    }
                    Topic::DbNameRequest => {
                        if let Some(req_payload) = deserialize_db_name_request_payload(&payload) {
                            if let Some(player_name) = player_map.get(&req_payload.player_id) {
                                let response_payload =
                                    serialize_db_name_response_payload(&DbNameResponsePayload {
                                        username: player_name.clone(),
                                    });
                                let target_topic = Topic::DbNameResponse(req_payload.player_id);
                                if let Err(e) =
                                    client.publish_raw(target_topic, &response_payload).await
                                {
                                    tracing::error!("Failed to publish DbNameResponse: {:?}", e);
                                } else {
                                    tracing::info!(
                                        "Sent DbNameResponse for player_id={} to response topic",
                                        req_payload.player_id
                                    );
                                }
                            } else {
                                tracing::warn!(
                                    "Name requested for unknown player_id={}",
                                    req_payload.player_id
                                );
                            }
                        } else {
                            tracing::error!("Failed to deserialize DbNameRequestPayload");
                        }
                    }
                    Topic::DbQuery => {
                        if let Some(query_payload) = deserialize_db_query_payload(&payload) {
                            if let Some(player_name) = player_map.get(&query_payload.player_id) {
                                let player_name_clone = player_name.clone();
                                let supabase_clone = supabase.clone();

                                tokio::spawn(async move {
                                    tracing::info!(
                                        "Updating player position: id={}, name={}, x={}, y={}",
                                        query_payload.player_id,
                                        player_name_clone,
                                        query_payload.x,
                                        query_payload.y
                                    );
                                    if let Err(e) = supabase_clone
                                        .update_player_position(
                                            &player_name_clone,
                                            query_payload.x,
                                            query_payload.y,
                                        )
                                        .await
                                    {
                                        tracing::error!(
                                            "Failed to update player position for {}: {:?}",
                                            player_name_clone,
                                            e
                                        );
                                    }
                                });
                            } else {
                                tracing::warn!(
                                    "Attempted to update position for unregistered player_id={}",
                                    query_payload.player_id
                                );
                            }
                        } else {
                            tracing::error!("Failed to deserialize DbQueryPayload");
                        }
                    }
                    _ => {}
                }
            }
        }

        tick.tick().await;
    }
}
