use std::sync::Arc;
use std::sync::mpsc::Receiver;

use bevy::prelude::*;
use bevy::tasks::Task;
use futures_lite::future;
use quinn::{ClientConfig, Endpoint};
use tokio::runtime::Runtime;

use common::packets::PositionBatch;

use super::{GameSession, GameState};

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        // Create one persistent Tokio runtime for all quinn operations.
        let rt = Runtime::new().expect("failed to create Tokio runtime for QUIC");
        app.insert_resource(QuicRuntime(Arc::new(rt)))
            .add_message::<PositionBatchReceived>()
            .add_systems(OnEnter(GameState::Connecting), start_connect)
            .add_systems(
                Update,
                poll_connect_task.run_if(in_state(GameState::Connecting)),
            )
            .add_systems(Update, receive_packets.run_if(in_state(GameState::InGame)));
    }
}

/// A single persistent Tokio runtime that owns all quinn async tasks.
/// Keeping it alive means the quinn endpoint driver keeps running.
#[derive(Resource, Clone)]
#[allow(dead_code)] // inner runtime kept alive intentionally; used when real QUIC is wired up
struct QuicRuntime(Arc<Runtime>);

// ---------------------------------------------------------------------------
// Connection task
// ---------------------------------------------------------------------------

#[derive(Component)]
struct ConnectTask(Task<Result<u32, String>>);

/// The entity_id the server assigned to this client.
#[derive(Resource, Clone, Copy)]
pub struct MyEntityId(pub u32);

/// Bevy resource holding the active QUIC connection to the gatekeeper.
#[derive(Resource)]
#[allow(dead_code)]
pub struct ServerConnection(pub quinn::Connection);

/// Channel receiver fed by the background datagram receive task.
/// Wrapped in Mutex so it satisfies Bevy's Resource: Sync bound.
#[derive(Resource)]
pub struct DatagramReceiver(std::sync::Mutex<Receiver<Vec<u8>>>);

fn start_connect(mut commands: Commands, session: Res<GameSession>, _rt: Res<QuicRuntime>) {
    let player_id = session.player_id.clone();

    // TODO: Replace with a real QUIC connection to the dedicated game server
    // at session.server_ip:session.server_port once it is implemented.
    // For now, derive a deterministic entity_id from the player_id and skip
    // the network round-trip entirely.
    let task = bevy::tasks::AsyncComputeTaskPool::get().spawn(async move {
        let entity_id: u32 = player_id
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_add(b as u32));
        Ok::<u32, String>(entity_id)
    });

    commands.spawn(ConnectTask(task));
}

fn poll_connect_task(
    mut commands: Commands,
    mut tasks: Query<(Entity, &mut ConnectTask)>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    for (entity, mut task) in &mut tasks {
        if let Some(result) = future::block_on(future::poll_once(&mut task.0)) {
            commands.entity(entity).despawn();
            match result {
                Ok(entity_id) => {
                    commands.insert_resource(MyEntityId(entity_id));
                    next_state.set(GameState::InGame);
                    tracing::info!(
                        entity_id,
                        "Session ready — entering game (server connection stubbed)"
                    );
                }
                Err(e) => {
                    tracing::error!("Session setup failed: {e}");
                    next_state.set(GameState::Login);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Packet receive loop
// ---------------------------------------------------------------------------

/// Message emitted when a fresh `PositionBatch` arrives from the server.
#[derive(Message)]
pub struct PositionBatchReceived(pub PositionBatch);

fn receive_packets(
    receiver: Option<Res<DatagramReceiver>>,
    mut batch_writer: MessageWriter<PositionBatchReceived>,
) {
    let Some(receiver) = receiver else { return };
    let Ok(rx) = receiver.0.lock() else { return };
    while let Ok(data) = rx.try_recv() {
        if let Ok(batch) = bitcode::decode::<PositionBatch>(&data) {
            batch_writer.write(PositionBatchReceived(batch));
        }
    }
}

// ---------------------------------------------------------------------------
// TLS — skip cert verification in debug builds
// ---------------------------------------------------------------------------

#[allow(dead_code)] // called when real QUIC connection to the game server is implemented
fn make_client_endpoint() -> anyhow::Result<Endpoint> {
    let crypto = make_client_crypto()?;
    let mut transport = quinn::TransportConfig::default();
    // Send a QUIC PING every 10 s to keep the connection alive.
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(10)));
    // Allow up to 60 s of silence before declaring the connection dead.
    transport.max_idle_timeout(Some(std::time::Duration::from_secs(60).try_into()?));
    let mut client_cfg = ClientConfig::new(Arc::new(crypto));
    client_cfg.transport_config(Arc::new(transport));
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_cfg);
    Ok(endpoint)
}

#[allow(dead_code)] // called by make_client_endpoint
fn make_client_crypto() -> anyhow::Result<quinn::crypto::rustls::QuicClientConfig> {
    // The server generates a self-signed cert at runtime via rcgen.
    // Until a proper PKI with a shared CA is in place, skip verification.
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    quinn::crypto::rustls::QuicClientConfig::try_from(crypto).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Dev cert verifier
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)] // used by make_client_crypto
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer,
        _intermediates: &[rustls::pki_types::CertificateDer],
        _server_name: &rustls::pki_types::ServerName,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
