use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use std::time::Duration;
use uuid::Uuid;

use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream, GameStreamReliability};

use crate::broker_messages::{BrokerMessage, SendingSystem};
use crate::topics::Topic;

/// A generic client for interacting with the messaging Broker.
pub struct BrokerClient {
    peer: GamePeer,
    connection: GameConnection,
    stream: GameStream,
    system_type: SendingSystem,
    client_id: Uuid,
}

impl BrokerClient {
    /// Connects to the broker, sets up streams, and registers the system instance.
    pub async fn connect(host: &str, port: u16, system_type: SendingSystem) -> Result<Self> {
        tracing::info!(
            "Connecting to broker at {}:{} as {:?}",
            host,
            port,
            system_type
        );

        let mut peer = GamePeer::new(QuicBackend::new());
        peer.connect(host, port)
            .context("Failed to initiate connection to broker")?;

        // 1. Wait for connection establishing
        let connection = Self::wait_for_connection(&mut peer).await?;

        // 2. Open reliable control stream
        peer.create_stream(connection, GameStreamReliability::Reliable)
            .context("Failed to request broker reliable stream")?;
        let stream = Self::wait_for_stream(&mut peer, connection).await?;

        let client = Self {
            peer,
            connection,
            stream,
            system_type,
            client_id: connection.connection_id.clone(),
        };

        // 3. Register our system ID with the broker
        client.announce_connect().await?;
        tracing::info!("Broker client successfully connected and registered.");

        Ok(client)
    }

    /// Announces our system identifier and type to the Broker.
    async fn announce_connect(&self) -> Result<()> {
        let bytes = BrokerMessage::serialize_connect(self.client_id, self.system_type.clone());
        self.send_bytes(bytes, "Connect").await
    }

    /// Subscribes this client to a specific Topic.
    pub async fn subscribe(&self, topic: Topic) -> Result<()> {
        let bytes = BrokerMessage::serialize_subscribe(self.client_id, topic.to_bytes());
        self.send_bytes(bytes, "Subscribe").await
    }

    /// Unsubscribes this client from a specific Topic.
    pub async fn unsubscribe(&self, topic: Topic) -> Result<()> {
        let bytes = BrokerMessage::serialize_unsubscribe(self.client_id, topic.to_bytes());
        self.send_bytes(bytes, "Unsubscribe").await
    }

    /// Publishes a raw byte payload to a given Topic.
    pub async fn publish_raw(&self, topic: Topic, payload: &[u8]) -> Result<()> {
        let bytes = BrokerMessage::serialize_publish(topic.to_bytes(), payload);
        self.send_bytes(bytes, "Publish").await
    }

    /// Helper to send bytes down our primary reliable control stream.
    async fn send_bytes(&self, bytes: Vec<u8>, context: &str) -> Result<()> {
        self.peer
            .send(&self.connection, &self.stream, Bytes::from(bytes))
            .with_context(|| format!("Broker client failed to send [{}] message", context))?;
        Ok(())
    }

    /// Polls incoming network traffic and filters out incoming broadcast messages.
    /// Returns the active `Topic` along with its raw payload vector.
    pub fn poll_broadcasts(&mut self) -> Result<Vec<(Topic, Vec<u8>)>> {
        let mut messages = Vec::new();

        while let Ok(Some(event)) = GamePeer::poll(&mut self.peer) {
            match event {
                GameNetworkEvent::Message { data, .. } => {
                    if let Some(BrokerMessage::Broadcast { topic, payload }) =
                        BrokerMessage::deserialize(&data)
                    {
                        messages.push((Topic::from_bytes(topic), payload));
                    }
                }
                GameNetworkEvent::Disconnected(_) => {
                    return Err(anyhow!("Disconnected from Broker unexpectedly."));
                }
                GameNetworkEvent::Error { inner, .. } => {
                    tracing::error!("Broker network layer error: {}", inner);
                }
                _ => {}
            }
        }

        Ok(messages)
    }

    // --- Private Connection Helpers ---

    async fn wait_for_connection(peer: &mut GamePeer) -> Result<GameConnection> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            while let Ok(Some(event)) = GamePeer::poll(peer) {
                if let GameNetworkEvent::Connected(conn) = event {
                    return Ok(conn);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("Timeout waiting for Broker network connection"));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn wait_for_stream(
        peer: &mut GamePeer,
        target_conn: GameConnection,
    ) -> Result<GameStream> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            while let Ok(Some(event)) = GamePeer::poll(peer) {
                if let GameNetworkEvent::StreamCreated(conn, stream) = event {
                    if conn == target_conn && stream.is_reliable() {
                        return Ok(stream);
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("Timeout waiting for Broker reliable stream setup"));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
