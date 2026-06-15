use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::broker_messages::{BrokerMessage, SendingSystem};
use common::topics::{
    serialize_starting_position_payload, StartingPositionPayload, Topic,
};
use game_sockets::protocols::QuicBackend;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream, GameStreamReliability};
use tokio::sync::mpsc;
use uuid::Uuid;

/// An inbound broadcast received from the broker for this AI.
#[derive(Debug)]
pub struct InboundMessage {
    pub topic: [u8; 32],
    pub payload: Vec<u8>,
}

/// A live QUIC connection representing one AI entity on the broker.
pub struct AiClient {
    pub id: Uuid,
    peer: Arc<Mutex<GamePeer>>,
    conn: GameConnection,
    stream: GameStream,
    pub inbound_tx: mpsc::UnboundedSender<InboundMessage>,
}

impl AiClient {
    /// Connect to the broker, perform the handshake, and subscribe to AOI updates.
    /// Returns the client and the receiver end of the inbound message channel.
    pub async fn connect(
        id: Uuid,
        broker_host: &str,
        broker_port: u16,
        starting_position: [f64; 2],
    ) -> Result<(Self, mpsc::UnboundedReceiver<InboundMessage>), String> {
        let peer = GamePeer::new(QuicBackend::new());
        peer.connect(broker_host, broker_port)
            .map_err(|e| format!("connect error: {e:?}"))?;

        let peer = Arc::new(Mutex::new(peer));
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();

        let (conn, stream) = await_handshake(Arc::clone(&peer)).await?;

        let client = Self { id, peer, conn, stream, inbound_tx };

        client.send_raw(BrokerMessage::serialize_connect(id, SendingSystem::AiService));
        client.send_raw(BrokerMessage::serialize_subscribe(
            id,
            Topic::EntityPositionUpdate(id).to_bytes(),
        ));
        client.send_raw(BrokerMessage::serialize_subscribe(
            id,
            Topic::AuthorityDebugPacket(id).to_bytes(),
        ));
        client.send_raw(BrokerMessage::serialize_publish(
            Topic::PlayerStartingPosition.to_bytes(),
            &serialize_starting_position_payload(&StartingPositionPayload {
                connection_id: id,
                position: starting_position,
            }),
        ));

        Ok((client, inbound_rx))
    }

    /// Poll the peer for incoming broadcasts and forward them to the inbound channel.
    pub fn poll(&self) {
        let Ok(mut peer) = self.peer.lock() else { return };
        while let Ok(Some(event)) = peer.poll() {
            if let GameNetworkEvent::Message { data, .. } = event {
                if let Some(BrokerMessage::Broadcast { topic, payload }) =
                    BrokerMessage::deserialize(&data)
                {
                    let _ = self.inbound_tx.send(InboundMessage { topic, payload });
                }
            }
        }
    }

    /// Publish a message to the broker on behalf of this AI.
    pub fn publish(&self, topic: [u8; 32], payload: &[u8]) {
        self.send_raw(BrokerMessage::serialize_publish(topic, payload));
    }

    fn send_raw(&self, bytes: Vec<u8>) {
        let Ok(peer) = self.peer.lock() else { return };
        if let Err(e) = peer.send(&self.conn, &self.stream, bytes.into()) {
            tracing::warn!("AiClient {}: send error: {e:?}", self.id);
        }
    }
}

/// A special system-wide client that listens to quadtree topologies to guide spawning.
pub struct MasterClient {
    pub id: Uuid,
    peer: Arc<Mutex<GamePeer>>,
    conn: GameConnection,
    stream: GameStream,
    pub inbound_tx: mpsc::UnboundedSender<InboundMessage>,
}

impl MasterClient {
    pub async fn connect(
        id: Uuid,
        broker_host: &str,
        broker_port: u16,
    ) -> Result<(Self, mpsc::UnboundedReceiver<InboundMessage>), String> {
        let peer = GamePeer::new(QuicBackend::new());
        peer.connect(broker_host, broker_port)
            .map_err(|e| format!("connect error: {e:?}"))?;

        let peer = Arc::new(Mutex::new(peer));
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();

        let (conn, stream) = await_handshake(Arc::clone(&peer)).await?;

        let client = Self { id, peer, conn, stream, inbound_tx };

        client.send_raw(BrokerMessage::serialize_connect(id, SendingSystem::AiService));
        client.send_raw(BrokerMessage::serialize_subscribe(
            id,
            Topic::QuadtreeBoundariesUpdate.to_bytes(),
        ));

        Ok((client, inbound_rx))
    }

    pub fn poll(&self) {
        let Ok(mut peer) = self.peer.lock() else { return };
        while let Ok(Some(event)) = peer.poll() {
            if let GameNetworkEvent::Message { data, .. } = event {
                if let Some(BrokerMessage::Broadcast { topic, payload }) =
                    BrokerMessage::deserialize(&data)
                {
                    let _ = self.inbound_tx.send(InboundMessage { topic, payload });
                }
            }
        }
    }

    fn send_raw(&self, bytes: Vec<u8>) {
        let Ok(peer) = self.peer.lock() else { return };
        if let Err(e) = peer.send(&self.conn, &self.stream, bytes.into()) {
            tracing::warn!("MasterClient {}: send error: {e:?}", self.id);
        }
    }
}

/// Standalone handshake routine utilized by both AiClient and MasterClient.
pub async fn await_handshake(
        peer: Arc<Mutex<GamePeer>>,
    ) -> Result<(GameConnection, GameStream), String> {
        loop {
            let event = {
                let Ok(mut p) = peer.lock() else { continue };
                p.poll().ok().flatten()
            };
            match event {
                Some(GameNetworkEvent::Connected(conn)) => {
                    let Ok(mut p) = peer.lock() else { continue };
                    p.create_stream(conn, GameStreamReliability::Reliable)
                        .map_err(|e| format!("stream error: {e:?}"))?;
                }
                Some(GameNetworkEvent::StreamCreated(conn, stream)) if stream.is_reliable() => {
                    return Ok((conn, stream));
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Holds all active AI clients, keyed by their UUID.
#[derive(Default)]
pub struct ClientPool {
    pub clients: HashMap<Uuid, AiClient>,
}

impl ClientPool {
    /// Poll every client for inbound messages.
    pub fn poll_all(&self) {
        for client in self.clients.values() {
            client.poll();
        }
    }
}