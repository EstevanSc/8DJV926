//! QUIC server for orchestrator to listen for shard updates from quadtree.
//!
//! Accepts incoming QUIC connections from the quadtree and receives
//! ShardData updates as JSON messages.

use anyhow::{Context, Result};
use common::ShardData;
use quinn::Endpoint;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

/// Create a QUIC server config with self-signed cert (generated at runtime, simple approach).
fn make_server_config() -> quinn::ServerConfig {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
        .expect("Failed to generate self-signed certificate");
    
    let cert_der = cert.serialize_der().expect("Failed to serialize cert");
    let priv_key = cert.serialize_private_key_der();
    let priv_key = rustls::PrivateKey(priv_key);
    let cert_chain = vec![rustls::Certificate(cert_der)];
    
    quinn::ServerConfig::with_single_cert(cert_chain, priv_key)
        .expect("Failed to create QUIC server config")
}

/// Message containing shard updates from quadtree.
#[derive(Debug, Clone)]
pub struct ShardUpdateMessage {
    pub shard_data: Vec<ShardData>,
}

/// Start the QUIC server on the given port and return a channel for shard updates.
pub async fn start_quic_server(
    port: u16,
) -> Result<mpsc::Receiver<ShardUpdateMessage>> {
    let (tx, rx) = mpsc::channel(100);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Starting QUIC server on {}", addr);

    // Create server config (with self-signed cert, no client verification)
    let server_config = make_server_config();

    // Bind endpoint on a UDP socket
    let socket = std::net::UdpSocket::bind(addr)
        .context("Failed to bind UDP socket")?;
    socket.set_nonblocking(true)
        .context("Failed to set socket non-blocking")?;
    
    let runtime = Arc::new(quinn::TokioRuntime);
    let endpoint = Endpoint::new(Default::default(), Some(server_config), socket, runtime)
        .context("Failed to create QUIC endpoint")?;

    // Spawn a task to handle incoming connections
    tokio::spawn(async move {
        handle_quic_connections(endpoint, tx).await;
    });

    Ok(rx)
}

/// Handle incoming QUIC connections and receive shard updates.
async fn handle_quic_connections(endpoint: Endpoint, tx: mpsc::Sender<ShardUpdateMessage>) {
    while let Some(conn) = endpoint.accept().await {
        let tx = tx.clone();

        tokio::spawn(async move {
            match conn.await {
                Ok(connection) => {
                    info!("QUIC connection established from {:?}", connection.remote_address());
                    handle_quic_connection(connection, tx).await;
                }
                Err(e) => {
                    tracing::error!("Failed to establish QUIC connection: {}", e);
                }
            }
        });
    }
}

/// Handle a single QUIC connection, receiving messages until it closes.
async fn handle_quic_connection(
    connection: quinn::Connection,
    tx: mpsc::Sender<ShardUpdateMessage>,
) {
    loop {
        match connection.accept_uni().await {
            Ok(recv) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_quic_stream(recv, tx).await {
                        tracing::error!("Error handling QUIC stream: {}", e);
                    }
                });
            }
            Err(quinn::ConnectionError::ApplicationClosed(_)) => {
                info!("QUIC connection closed by remote");
                break;
            }
            Err(e) => {
                tracing::error!("Error accepting QUIC stream: {}", e);
                break;
            }
        }
    }
}

/// Handle a single QUIC stream, reading and parsing JSON shard data.
async fn handle_quic_stream(
    mut recv: quinn::RecvStream,
    tx: mpsc::Sender<ShardUpdateMessage>,
) -> Result<()> {
    let mut buf = Vec::new();

    // Read all data from the stream
    while let Ok(Some(chunk)) = recv.read_chunk(65536, true).await {
        buf.extend_from_slice(&chunk.bytes);
    }

    // Parse JSON
    let json_str = String::from_utf8(buf).context("Invalid UTF-8 in QUIC message")?;
    let shard_data: Vec<ShardData> =
        serde_json::from_str(&json_str).context("Failed to parse shard data JSON")?;

    tracing::debug!("Received shard update from quadtree: {} shards", shard_data.len());

    // Send to handler
    tx.send(ShardUpdateMessage { shard_data })
        .await
        .context("Failed to send shard update message")?;

    Ok(())
}
