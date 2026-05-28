//! QUIC client wrapper used by the quadtree for separate orchestrator and broker connections.

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use common::ShardData;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream};
use std::time::Duration;

/// Quadtree QUIC connection that maintains a single target connection.
pub struct QuicClient {
    peer: GamePeer,
    connection: GameConnection,
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

        tracing::info!("{} QUIC link connected (id={:?})", label, connection.connection_id);

        Ok(Self {
            peer,
            connection,
            label: label.to_string(),
        })
    }

    pub async fn connect_orchestrator(host: &str, port: u16) -> Result<Self> {
        Self::connect("orchestrator", host, port).await
    }

    pub async fn connect_broker(host: &str, port: u16) -> Result<Self> {
        Self::connect("broker", host, port).await
    }

    /// Send shard data to the orchestrator using the shared binary schema.
    pub async fn send_shard_data(&self, shard_data: &[ShardData]) -> Result<()> {
        let payload = ShardData::encode_batch(shard_data)
            .context("Failed to encode shard data payload")?;
        let stream = GameStream::from(0);

        self.peer
            .send(&self.connection, &stream, Bytes::from(payload))
            .with_context(|| format!("Failed to send shard data on {} link", self.label))?;

        tracing::debug!("Sent shard data to orchestrator: {} shards", shard_data.len());

        Ok(())
    }
}
