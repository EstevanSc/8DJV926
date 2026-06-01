use std::collections::{HashMap, HashSet};
use bytes::Bytes;
use game_sockets;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameSocketError, GameStream, GameStreamReliability};
use game_sockets::protocols::QuicBackend;
use common::broker_messages::BrokerMessage;
use common::topics::Topic;
use common::topics::{DisconnectPayload, serialize_disconnect_payload};

pub struct BrokerConfig {
    pub ip: String,
    pub public_ip: String,
    pub port: u16,
}
impl BrokerConfig {
    pub fn from_env() -> Self {
        let port = std::env::var("BROKER_PORT")
            .unwrap_or_else(|_| "7776".to_string())
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
    reverse_uuid_map: HashMap<GameConnection, uuid::Uuid>, 
    broker_stream: GameStream // Inverse map for removal during disconnect
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
            broker_stream: GameStream::new(1, GameStreamReliability::Reliable), // Control stream for broker-originated messages
        }
    }
    pub fn receive_packets(&mut self) {
        while let Ok(Some(event)) = self.peer.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("Connected! Connection id: {:?}", conn.connection_id);
                }
                GameNetworkEvent::Disconnected(conn) => {

                    println!("Disconnected! Connection id: {:?}", conn.connection_id);
                    if let Some(entity_id) = self.reverse_uuid_map.get(&conn).cloned() {
                        let disconnected_payload = serialize_disconnect_payload(&DisconnectPayload { entity_id });
                        let topic = Topic::Disconnect(entity_id).to_bytes();
                        self.publish(topic, disconnected_payload, self.broker_stream.clone());
                    }

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
                let topic_desc = Topic::from_bytes(topic);
                println!("Broker: Subscribe request - client_id={}, topic={:?}", client_id, topic_desc);
                self.subscriptions.entry(topic).or_default().insert(client_id);
            }
            BrokerMessage::Unsubscribe { client_id, topic } => {
                let topic_desc = Topic::from_bytes(topic);
                println!("Broker: Unsubscribe request - client_id={}, topic={:?}", client_id, topic_desc);
                self.subscriptions.entry(topic).or_default().remove(&client_id);
            }
            BrokerMessage::Publish {topic, payload} => {
                //let topic_desc = Topic::from_bytes(topic);
                // println!("Broker: Publish received - topic={:?}, payload_len={}", topic_desc, payload.len());
                self.publish(topic, payload, stream);
            }
            BrokerMessage::Connect {client_id} => {
                println!("Broker: Connect from client_id={} (conn_id={:?})", client_id, connection.connection_id);
                self.uuid_map.insert(client_id, connection.clone());
                self.reverse_uuid_map.insert(connection, client_id.clone());
            }
            _ => {} // The broker shouldn't receive broadcast messages
        }
    }

    fn publish(&mut self, topic: [u8;32], payload: Vec<u8>, stream: GameStream) {
        if let Some(subscribers) = self.subscriptions.get(&topic) {
            //println!("Broker: publishing to {} subscribers", subscribers.len());
            let broadcast_bytes = BrokerMessage::serialize_broadcast(topic, &payload);
            let bytes_payload = Bytes::from(broadcast_bytes);

            let topic_desc = Topic::from_bytes(topic);
            match topic_desc {
                Topic::Input(a) | Topic::ShardSnapshot(a) => {}
                _ => {
                    println!("Broker: publishing message {:?} to {} subscribers", topic_desc, subscribers.len());
                }
            }

            for subscriber_uuid in subscribers {
                let target_conn = self.uuid_map.get_mut(&subscriber_uuid);
                match target_conn 
                {
                    Some(target_conn) => {
                        if let Err(e) = self.peer.send(&target_conn, &stream, bytes_payload.clone()) {
                            eprintln!("Failed to forward publish to {}: {:?}", subscriber_uuid, e);
                        }
                    }
                    None => {
                        eprintln!("No active connection for subscriber {} - can't deliver publish", subscriber_uuid);
                    }
                }
            }
        } else {
            println!("Broker: publish had no subscribers for topic {:?}", Topic::from_bytes(topic));
        }
    }
}