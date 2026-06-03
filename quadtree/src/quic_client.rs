//! QUIC client wrapper used by the quadtree for separate orchestrator and broker connections.

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use common::topics::Topic;
use common::{Boundary};
use common::broker_messages::{BrokerMessage, SendingSystem};
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream, GameStreamReliability};
use std::time::Duration;
use uuid::Uuid;

/// Quadtree QUIC connection that maintains a single target connection.
pub struct QuicClient {
    peer: GamePeer,
    connection: GameConnection,
    control_stream: GameStream,
    label: String,
}

impl QuicClient {
    async fn wait_for_connection(peer: &mut GamePeer, label: &str) -> Result<GameConnection> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        loop {
            while let Ok(Some(event)) = GamePeer::poll(peer) {
                match event {
                    GameNetworkEvent::Connected(connection) => {
                        return Ok(connection);
                    }
                    GameNetworkEvent::Disconnected(connection) => {
                        return Err(anyhow!(
                            "{} connection closed before it became ready ({:?})",
                            label,
                            connection.connection_id
                        ));
                    }
                    GameNetworkEvent::Error { inner, .. } => {
                        return Err(anyhow!("{} connection error: {}", label, inner));
                    }
                    _ => {}
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("{} connection timed out", label));
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn connect(label: &str, host: &str, port: u16) -> Result<Self> {
        tracing::info!("Connecting {} QUIC link to {}:{}", label, host, port);

        let peer = GamePeer::new(QuicBackend::new());
        peer.connect(host, port)
            .with_context(|| format!("Failed to start {} QUIC connection", label))?;

        let mut peer = peer;
        let connection = Self::wait_for_connection(&mut peer, label).await?;
        peer.create_stream(connection, GameStreamReliability::Reliable)
            .with_context(|| format!("Failed to create {} control stream", label))?;
        let control_stream = Self::wait_for_reliable_stream(&mut peer, label, connection).await?;

        tracing::info!("{} QUIC link connected (id={:?})", label, connection.connection_id);

        Ok(Self {
            peer,
            connection,
            control_stream,
            label: label.to_string(),
        })
    }

    async fn wait_for_reliable_stream(
        peer: &mut GamePeer,
        label: &str,
        connection: GameConnection,
    ) -> Result<GameStream> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

        loop {
            while let Ok(Some(event)) = GamePeer::poll(peer) {
                match event {
                    GameNetworkEvent::StreamCreated(stream_connection, stream)
                        if stream_connection == connection && stream.is_reliable() =>
                    {
                        return Ok(stream);
                    }
                    GameNetworkEvent::Disconnected(stream_connection)
                        if stream_connection == connection =>
                    {
                        return Err(anyhow!(
                            "{} control stream closed before it became ready ({:?})",
                            label,
                            stream_connection.connection_id
                        ));
                    }
                    GameNetworkEvent::Error {
                        connection: stream_connection,
                        inner,
                    } if stream_connection == connection => {
                        return Err(anyhow!(
                            "{} control stream setup error: {}",
                            label,
                            inner
                        ));
                    }
                    _ => {}
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("{} control stream setup timed out", label));
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn connect_orchestrator(host: &str, port: u16) -> Result<Self> {
        Self::connect("orchestrator", host, port).await
    }

    pub async fn connect_broker(host: &str, port: u16) -> Result<Self> {
        Self::connect("broker", host, port).await
    }

    pub fn poll(&mut self) -> Result<Option<GameNetworkEvent>> {
        GamePeer::poll(&mut self.peer).map_err(|e| anyhow!("{} link poll failed: {}", self.label, e))
    }

    pub fn connection_id(&self) -> Uuid {
        self.connection.connection_id
    }

    async fn send_bytes(&self, bytes: Vec<u8>, context: &str) -> Result<()> {
        self.peer
            .send(&self.connection, &self.control_stream, Bytes::from(bytes))
            .with_context(|| format!("Failed to send {} on {} link", context, self.label))?;

        Ok(())
    }

    pub async fn subscribe(&self, client_id: Uuid, topic: Topic) -> Result<()> {
        self.send_bytes(
            BrokerMessage::serialize_subscribe(client_id, topic.to_bytes()),
            "subscribe",
        )
        .await
    }

    pub async fn unsubscribe(&self, client_id: Uuid, topic: Topic) -> Result<()> {
        self.send_bytes(
            BrokerMessage::serialize_unsubscribe(client_id, topic.to_bytes()),
            "unsubscribe",
        )
        .await
    }

    pub async fn announce_connect(&self, client_id: Uuid) -> Result<()> {
        self.send_bytes(BrokerMessage::serialize_connect(client_id, SendingSystem::Quadtree), "connect").await
    }

    pub async fn publish(&self, topic: Topic, payload: &[u8]) -> Result<()> {
        self.send_bytes(
            BrokerMessage::serialize_publish(topic.to_bytes(), payload),
            "publish",
        )
        .await
    }

    /// Send boundaries to the orchestrator using the shared binary schema.
    pub async fn send_shard_data(&self, boundaries: &Vec<Boundary>) -> Result<()> {
        let payload: Vec<u8> = Boundary::encode_batch(boundaries)
            .context("Failed to encode boundaries payload")?;
        self.send_bytes(payload, "shard data").await?;

        tracing::debug!("Sent boundaries to orchestrator: {} boundaries", boundaries.len());

        Ok(())
    }
}
