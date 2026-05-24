use std::collections::{HashMap, HashSet};
use bytes::Bytes;
use game_sockets;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameSocketError, GameStream};
use game_sockets::protocols::QuicBackend;
use crate::messages::BrokerMessage;

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
    subscriptions: HashMap<[u8; 32], HashSet<u32>>,
    client_addr_map: HashMap<u32, uuid::Uuid>,  // Maps each client id to its corresponding address
    shard_addr_map: HashMap<u32, uuid::Uuid>,
    client_shard_map: HashMap<u32, u32>, // Maps each client id to its relevant shard's id
}

fn bind_socket(config: &BrokerConfig) -> Result<GamePeer, GameSocketError> {
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
    fn receive_packets(&mut self) {
        while let Ok(Some(event)) = self.peer.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("Connected! Client id: {:?}", conn.connection_id);
                }
                GameNetworkEvent::Disconnected(_) => {}
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

                // Map the client to its associated shard
                if let Some(shard_id) = extract_shard_id_from_topic(&topic) {
                    self.client_shard_map.insert(client_id, shard_id);
                }
            }
            BrokerMessage::Unsubscribe { client_id, topic } => {
                self.subscriptions.entry(topic).or_default().remove(&client_id);
            }
            BrokerMessage::Publish {topic, payload} => {
                // Add the shard to our address map
                if let Some(shard_id) = extract_shard_id_from_topic(&topic) {
                    self.shard_addr_map.insert(shard_id, connection.connection_id);
                }

                if let Some(subscribers) = self.subscriptions.get(&topic) {
                    let broadcast_bytes = BrokerMessage::serialize_broadcast(&payload);
                    let bytes_payload = Bytes::from(broadcast_bytes);

                    for subscriber in subscribers {
                        if let Some(subscriber_uuid) = self.client_addr_map.get(subscriber) {
                            let target_conn = GameConnection::from(*subscriber_uuid);
                            // Relay matching the incoming stream rules (e.g. Unreliable Datagram)
                            let _ = self.peer.send(&target_conn, &stream, bytes_payload.clone());
                        }
                        else {
                            eprintln!("Dropped snapshot: client with id {} unknown", subscriber);
                        }
                    }
                }
            }
            BrokerMessage::ClientInput {client_id, input: _} => {
                // Add the client to the address map
                self.client_addr_map.insert(client_id, connection.connection_id);

                if let Some(shard) = self.client_shard_map.get(&client_id) {
                    if let Some(shard_uuid) = self.shard_addr_map.get(&shard) {
                        // Redirect the input packet to the shard
                        let shard_conn = GameConnection::from(*shard_uuid);
                        let _ = self.peer.send(&shard_conn, &stream, data.clone());
                    }
                    else {
                        eprintln!("Dropped input: shard with id {} unknown", shard);
                    }
                }
                else {
                    eprintln!("Dropped input: No active shard assigned to handle client {}", client_id);
                }
            }
            _ => {} // The broker shouldn't receive broadcast messages
        }
    }
}

pub fn extract_shard_id_from_topic(topic: &[u8; 32]) -> Option<u32> {
    let topic_str = match std::str::from_utf8(topic) {
        Ok(s) => s.trim_matches('\0').trim(),
        Err(_) => return None,
    };

    if let Some(id_str) = topic_str.strip_prefix("shard:") {
        id_str.parse::<u32>().ok()
    } else {
        None
    }
}