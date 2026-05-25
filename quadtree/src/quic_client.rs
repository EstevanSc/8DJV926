//! QUIC client for quadtree to send shard updates to orchestrator.
//!
//! Establishes a persistent connection to the orchestrator and sends
//! ShardData updates as JSON messages over QUIC unidirectional streams.

use anyhow::{anyhow, Context, Result};
use common::ShardData;
use rustls::client::{ServerCertVerified, ServerCertVerifier};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Minimal server cert verifier that skips all verification for local dev.
#[derive(Debug)]
struct SkipVerification;

impl ServerCertVerifier for SkipVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}

/// Create a QUIC client config (skips all server cert verification).
fn make_client_config() -> quinn::ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(SkipVerification))
        .with_no_client_auth();

    quinn::ClientConfig::new(Arc::new(crypto))
}

/// Quadtree QUIC client that maintains a connection to the orchestrator.
pub struct QuicClient {
    _endpoint: quinn::Endpoint,
    connection: Arc<Mutex<Option<quinn::Connection>>>,
}

impl QuicClient {
    /// Create a new QUIC client and connect to the orchestrator.
    pub async fn new(orchestrator_host: &str, orchestrator_port: u16) -> Result<Self> {
        // Resolve orchestrator address
        let orchestrator_addr = format!("{}:{}", orchestrator_host, orchestrator_port)
            .parse::<SocketAddr>()
            .context("Failed to parse orchestrator address")?;

        tracing::info!("Quadtree QUIC client connecting to {}", orchestrator_addr);

        // Create a QUIC client configuration (skips server cert verification)
        let client_config = make_client_config();

        // Create endpoint on a local address (OS will assign port)
        let socket = std::net::UdpSocket::bind("[::]:0")
            .context("Failed to bind UDP socket")?;
        socket.set_nonblocking(true)
            .context("Failed to set socket non-blocking")?;
        
        let runtime = Arc::new(quinn::TokioRuntime);
        let mut endpoint = quinn::Endpoint::new(Default::default(), None, socket, runtime)
            .context("Failed to create QUIC endpoint")?;
        
        endpoint.set_default_client_config(client_config);

        // Connect to orchestrator
        let connection = endpoint
            .connect(orchestrator_addr, "orchestrator")
            .context("Failed to connect to orchestrator")?
            .await
            .context("Connection to orchestrator failed")?;

        tracing::info!("Quadtree QUIC client connected to orchestrator");

        Ok(QuicClient {
            _endpoint: endpoint,
            connection: Arc::new(Mutex::new(Some(connection))),
        })
    }

    /// Send shard data to the orchestrator as JSON.
    pub async fn send_shard_data(&self, shard_data: &[ShardData]) -> Result<()> {
        let json_payload = serde_json::to_string(shard_data)
            .context("Failed to serialize shard data to JSON")?;

        let mut conn_lock = self.connection.lock().await;

        // Check if connection is alive; reconnect if needed
        if let Some(conn) = conn_lock.as_ref() {
            if conn.close_reason().is_some() {
                tracing::warn!("Connection to orchestrator was closed, reconnecting...");
                *conn_lock = None;
            }
        }

        let conn = conn_lock
            .as_ref()
            .ok_or_else(|| anyhow!("Connection is unavailable"))?;

        // Open a unidirectional stream and send data
        let mut send = conn
            .open_uni()
            .await
            .context("Failed to open unidirectional stream")?;

        send.write_all(json_payload.as_bytes())
            .await
            .context("Failed to write to QUIC stream")?;

        send.finish()
            .await
            .context("Failed to finish QUIC stream")?;

        tracing::debug!("Sent shard data to orchestrator: {} shards", shard_data.len());

        Ok(())
    }
}
