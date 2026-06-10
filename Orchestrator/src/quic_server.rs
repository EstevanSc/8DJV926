//! QUIC server for orchestrator to listen for shard updates from quadtree.
//!
//! Accepts incoming QUIC connections from the quadtree and receives
//! ShardData updates as binary-encoded messages over `game_sockets`.

use anyhow::{Context, Result};
use bytes::Bytes;
use common::Boundary;
use std::time::Duration;
use tokio::sync::mpsc;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameNetworkEvent, GamePeer};
use tracing::info;

/// Message containing shard updates from quadtree.
#[derive(Debug, Clone)]
pub struct ShardUpdateMessage {
    pub boundaries: Vec<Boundary>,
}

/// Start the QUIC server on the given port and return a channel for shard updates.
pub async fn start_quic_server(
    port: u16,
) -> Result<mpsc::Receiver<ShardUpdateMessage>> {
    let (tx, rx) = mpsc::channel(100);

    let peer = GamePeer::new(QuicBackend::new());
    peer.listen("0.0.0.0", port)
        .context("Failed to bind game_sockets QUIC listener")?;

    info!("Starting QUIC server on 0.0.0.0:{}", port);

    tokio::spawn(async move {
        handle_quic_connections(peer, tx).await;
    });

    Ok(rx)
}

/// Handle incoming QUIC connections and receive shard updates.
async fn handle_quic_connections(mut peer: GamePeer, tx: mpsc::Sender<ShardUpdateMessage>) {
    loop {
        while let Ok(Some(event)) = peer.poll() {
            match event {
                GameNetworkEvent::Connected(connection) => {
                    info!("QUIC connection established from {:?}", connection.connection_id);
                }
                GameNetworkEvent::Message { data, .. } => {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_quic_message(data, tx).await {
                            tracing::error!("Error handling QUIC message: {}", e);
                        }
                    });
                }
                GameNetworkEvent::Disconnected(connection) => {
                    info!("QUIC connection closed from {:?}", connection.connection_id);
                }
                GameNetworkEvent::Error { inner, .. } => {
                    tracing::error!("QUIC error: {}", inner);
                }
                _ => {}
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Handle a single QUIC message, reading and parsing binary shard data.
async fn handle_quic_message(
    data: Bytes,
    tx: mpsc::Sender<ShardUpdateMessage>,
) -> Result<()> {
    let boundaries = Boundary::decode_batch(&data)
        .context("Failed to decode boundaries payload")?;

    tracing::debug!("Received shard update from quadtree: {} boundaries", boundaries.len());

    // Send to handler
    tx.send(ShardUpdateMessage { boundaries })
        .await
        .context("Failed to send shard update message")?;

    Ok(())
}
