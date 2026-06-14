mod quic_client;
mod config;

use quic_client::QuicClient;
use crate::config::Config;

use std::time::{Duration};

use common::topics::{
      Topic, PathRequestPayload, PathResponsePayload, deserialize_path_request_payload, serialize_path_response_payload
};

use common::BrokerMessage;
use game_sockets::GameNetworkEvent;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

    let broker_client = match QuicClient::connect_broker(&config.broker_host, config.broker_port).await {
        Ok(client) => {
            tracing::info!("Connected to broker at {}:{}", config.broker_host, config.broker_port);
            Some(client)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to broker at {}:{}: {}", config.broker_host, config.broker_port, e);
            None
        }
    };

    run_main_loop(
        broker_client,
    )
    .await
}


async fn run_main_loop(
    broker_client: Option<QuicClient>,
) -> anyhow::Result<()> {

    let mut broker_client = broker_client;

    if let Some(client) = broker_client.as_ref() {
        let client_id = client.connection_id();
        client.announce_connect(client_id).await?;
        client.subscribe(client_id, Topic::PathRequest).await?;
        tracing::info!(
            "Subscribed pathing service to broker topics {:?} with client_id={}",
            Topic::PathRequest,
            client_id
        );
    }

    let mut tick = tokio::time::interval(Duration::from_millis(5));

    loop {
        tick.tick().await;
        
        if let Some(client) = broker_client.as_mut() {
            poll_quic_events(client, "broker").await?;
        }
    }
}

async fn poll_quic_events(
    broker: &mut QuicClient,
    label: &str,
) -> anyhow::Result<()> {
    while let Some(event) = broker.poll()? {
        match event {
            GameNetworkEvent::Message { data, connection, stream } => {
                tracing::debug!(
                    "{} link message: {} bytes from {:?} on stream {}",
                    label,
                    data.len(),
                    connection.connection_id,
                    stream.stream_id
                );

                handle_quic_message(&data, broker).await;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_quic_message(
    data: &[u8],
    broker: &QuicClient,
) {
    let Some(message) = BrokerMessage::deserialize(data) else {
        return;
    };

    match message {
        BrokerMessage::Broadcast { topic, payload }
        | BrokerMessage::Publish { topic, payload } => {
            match Topic::from_bytes(topic) {
                Topic::PathRequest => {
                    if let Some(path_request) = deserialize_path_request_payload(&payload) {
                        handle_path_request(&path_request, broker).await;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

async fn handle_path_request(
    request: &PathRequestPayload,
    broker: &QuicClient,
) {
    tracing::info!("Received path request for entity {} from {:?} to {:?}", request.entity_id, request.start, request.end);

    let topic = Topic::PathResponse(request.entity_id);

    //mock response for testing
    let response = PathResponsePayload {
        path: vec![
            request.start,
            [request.start[0], request.end[1]],
            [request.end[0], request.start[1]],
            request.end
        ],
    };

    let response_bytes = serialize_path_response_payload(&response);
    if let Err(e) = broker.publish(topic, &response_bytes).await {
        tracing::error!("Failed to publish path response for entity {}: {}", request.entity_id, e);
    }
}
