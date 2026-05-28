use std::collections::{HashMap, HashSet};
use bytes::Bytes;
use game_sockets;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameSocketError, GameStream};
use game_sockets::protocols::QuicBackend;
use common::broker_messages::BrokerMessage;

pub struct BrokerConfig {
    pub ip: String,
    pub public_ip: String,
    pub port: u16,
}
impl BrokerConfig {
    pub fn from_env() -> Self {
        let port = std::env::var("BROKER_PORT")
            .unwrap_or_else(|_| "7777".to_string())
            .parse::<u16>()
            .expect("Invalid BROKER_PORT");

        Self {
            ip: "0.0.0.0".to_string(),
            // DS_PUBLIC_IP is the address clients use to reach this server.
            // Set to "localhost" for local Docker dev (port-mapped to host).
            public_ip: std::env::var("BROKER_PUBLIC_IP").unwrap_or_else(|_| "localhost".to_string()),
            port,
        }
    }
}

pub struct BrokerState {
    peer: GamePeer,
    subscriptions: HashMap<[u8; 32], HashSet<uuid::Uuid>>,
    uuid_map: HashMap<uuid::Uuid, GameConnection>,
    reverse_uuid_map: HashMap<GameConnection, uuid::Uuid>,  // Inverse map for removal during disconnect
}

pub fn bind_socket(config: &BrokerConfig) -> Result<GamePeer, GameSocketError> {
    let peer: GamePeer = GamePeer::new(QuicBackend::new());
    let ip = &config.ip;
    let port = config.port;
    match peer.listen(ip, port) {
        Ok(_) => {
            println!("Listening on {}:{}", ip, port);
            Ok(peer)
        }
        Err(e) => {
            println!("Error listening on {}:{}", ip, port);
            Err(e)
        }
    }
}

impl BrokerState {
    pub fn new(peer: GamePeer) -> Self {
        Self {
            peer,
            subscriptions: HashMap::new(),
            uuid_map: HashMap::new(),
            reverse_uuid_map: HashMap::new(),
        }
    }
    pub fn receive_packets(&mut self) {
        while let Ok(Some(event)) = self.peer.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("Connected! Connection id: {:?}", conn.connection_id);
                }
                GameNetworkEvent::Disconnected(conn) => {
                    if let Some(id) = self.reverse_uuid_map.remove(&conn) {
                        self.uuid_map.remove(&id);

                        // Clean the subscriptions to prevent memory leak
                        for subscribers in self.subscriptions.values_mut() {
                            subscribers.remove(&id);
                        }
                    }
                }
                GameNetworkEvent::Message {
                    data,
                    connection,
                    stream,
                } => {
                    self.handle_message(data, connection, stream);
                }
                _ => {}
            }
        }
    }

    fn handle_message(&mut self, data: Bytes, connection: GameConnection, stream: GameStream) {
        let message = match BrokerMessage::deserialize(&data) {
            Some(msg) => msg,
            None => {
                eprintln!("Received malformed or unknown packet tag from {:?}", connection.connection_id);
                return;
            }
        };

        match message {
            BrokerMessage::Subscribe {client_id, topic} => {
                self.subscriptions.entry(topic).or_default().insert(client_id);
            }
            BrokerMessage::Unsubscribe { client_id, topic } => {
                self.subscriptions.entry(topic).or_default().remove(&client_id);
            }
            BrokerMessage::Publish {topic, payload} => {
                self.publish(topic, payload, stream);
            }
            BrokerMessage::Connect {client_id} => {
                self.uuid_map.insert(client_id, connection.clone());
                self.reverse_uuid_map.insert(connection, client_id.clone());
            }
            _ => {} // The broker shouldn't receive broadcast messages
        }
    }

    fn publish(&mut self, topic: [u8;32], payload: Vec<u8>, stream: GameStream) {
        if let Some(subscribers) = self.subscriptions.get(&topic) {
            let broadcast_bytes = BrokerMessage::serialize_broadcast(topic, &payload);
            let bytes_payload = Bytes::from(broadcast_bytes);

            for subscriber_uuid in subscribers {
                let target_conn = self.uuid_map.get_mut(&subscriber_uuid).unwrap();
                // Relay matching the incoming stream rules (e.g. Unreliable Datagram)
                let _ = self.peer.send(&target_conn, &stream, bytes_payload.clone());
            }
        }
    }
}