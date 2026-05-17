use std::sync::Mutex;

use bevy::prelude::*;
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

use common::packets::PositionBatch;

use super::{GameSession, GameState};

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PositionBatchReceived>()
            .add_systems(OnEnter(GameState::Connecting), start_connect)
            .add_systems(
                Update,
                (poll_net_events, tick_connect_timeout).run_if(in_state(GameState::Connecting)),
            )
            .add_systems(Update, receive_packets.run_if(in_state(GameState::InGame)));
    }
}

// ---------------------------------------------------------------------------
// Wire protocol — must match GameServer/src/messages.rs exactly.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
enum GameMessage {
    Join { username: String },
    Welcome { player_id: Uuid },
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Wraps the active `GamePeer` in a Mutex so it satisfies Bevy's Sync bound.
#[derive(Resource)]
pub struct ActivePeer(pub Mutex<GamePeer>);

/// Set to true once we have sent the `Join` message to the server.
#[derive(Resource, Default)]
struct JoinSent(bool);

/// Countdown started when we enter Connecting. If no `Connected` event arrives
/// before it expires the client returns to Login.
#[derive(Resource)]
struct ConnectTimeout(Timer);

/// The UUID assigned by the server in the `Welcome` message.
/// Also stored as a hash-derived `u32` for the position-interpolation system.
#[derive(Resource, Clone, Copy)]
pub struct MyEntityId(pub u32);

/// The active game-server connection handle. Inserted once the QUIC handshake
/// completes. Used by other systems (e.g. input) to send datagrams.
#[derive(Resource, Clone, Copy)]
pub struct ServerConn(pub GameConnection);

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

fn start_connect(mut commands: Commands, session: Res<GameSession>) {
    tracing::info!(
        "Connecting to game server at {}:{}",
        session.server_ip,
        session.server_port
    );

    let peer = GamePeer::new(QuicBackend::new());
    if let Err(e) = peer.connect(&session.server_ip, session.server_port) {
        tracing::error!("Failed to initiate QUIC connection: {e:?}");
    }

    commands.insert_resource(ActivePeer(Mutex::new(peer)));
    commands.insert_resource(JoinSent(false));
    commands.insert_resource(ConnectTimeout(Timer::from_seconds(10.0, TimerMode::Once)));
}

fn poll_net_events(
    mut commands: Commands,
    session: Res<GameSession>,
    peer_res: Option<ResMut<ActivePeer>>,
    mut join_sent: ResMut<JoinSent>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else { return };

    loop {
        let event = match peer.poll() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(_) => {
                tracing::error!("GamePeer backend thread crashed — returning to login");
                next_state.set(GameState::Login);
                break;
            }
        };
        match event {
            GameNetworkEvent::Connected(conn) => {
                tracing::info!("QUIC connected (id={:?}); sending Join", conn.connection_id);
                commands.insert_resource(ServerConn(conn));

                if !join_sent.0 {
                    let msg = GameMessage::Join {
                        username: session.username.clone(),
                    };
                    match wincode::serialize(&msg) {
                        Ok(data) => {
                            let stream = GameStream::from(0);
                            if let Err(e) = peer.send(&conn, &stream, data.into()) {
                                tracing::error!("Failed to send Join: {e:?}");
                            } else {
                                tracing::info!("Sent Join for '{}'", session.username);
                                join_sent.0 = true;
                            }
                        }
                        Err(e) => tracing::error!("Failed to serialize Join: {e:?}"),
                    }
                }
            }

            GameNetworkEvent::Message { data, .. } => {
                match wincode::deserialize::<GameMessage>(&data) {
                    Ok(GameMessage::Welcome { player_id }) => {
                        tracing::info!("Received Welcome — player_id={player_id}");
                        // Derive a u32 from the UUID for the position-interpolation system.
                        let entity_id = player_id
                            .as_bytes()
                            .iter()
                            .fold(0u32, |acc, &b| acc.wrapping_add(b as u32));
                        commands.insert_resource(MyEntityId(entity_id));
                        next_state.set(GameState::InGame);
                    }
                    Ok(other) => {
                        tracing::warn!("Unexpected message variant: {other:?}");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to decode server message: {e:?}");
                    }
                }
            }

            GameNetworkEvent::Disconnected(conn) => {
                tracing::warn!("Disconnected from server ({:?})", conn.connection_id);
                next_state.set(GameState::Login);
            }

            GameNetworkEvent::Error { inner, .. } => {
                tracing::error!("Network error: {inner}");
                next_state.set(GameState::Login);
            }

            _ => {}
        }
    }
}

fn tick_connect_timeout(
    mut commands: Commands,
    time: Res<Time>,
    mut timeout: ResMut<ConnectTimeout>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if timeout.0.tick(time.delta()).just_finished() {
        tracing::warn!("Connection timed out — returning to login");
        commands.remove_resource::<ConnectTimeout>();
        next_state.set(GameState::Login);
    }
}

// ---------------------------------------------------------------------------
// InGame packet receive (position batches from the server)
// ---------------------------------------------------------------------------

/// Message emitted when a fresh `PositionBatch` arrives from the server.
#[derive(Message)]
pub struct PositionBatchReceived(pub PositionBatch);

fn receive_packets(
    peer_res: Option<ResMut<ActivePeer>>,
    mut batch_writer: MessageWriter<PositionBatchReceived>,
) {
    let Some(peer_res) = peer_res else { return };
    let Ok(mut peer) = peer_res.0.lock() else { return };

    while let Ok(Some(event)) = peer.poll() {
        if let GameNetworkEvent::Message { data, .. } = event {
            if let Ok(batch) = bitcode::decode::<PositionBatch>(&data) {
                batch_writer.write(PositionBatchReceived(batch));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TLS — skip cert verification in debug builds
// ---------------------------------------------------------------------------
/*
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
*/
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
