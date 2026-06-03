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
    //pub public_ip: String,
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
            //public_ip: std::env::var("BROKER_PUBLIC_IP").unwrap_or_else(|_| "localhost".to_string()),
            port,
        }
    }
}

pub struct BrokerState {
    peer: GamePeer,
    subscriptions: HashMap<[u8; 32], HashSet<GameConnection>>,
    connections: HashSet<GameConnection>,
    broker_stream: GameStream,
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
            connections: HashSet::new(),
            broker_stream: GameStream::new(1, GameStreamReliability::Reliable),
        }
    }
    pub fn receive_packets(&mut self) {
        while let Ok(Some(event)) = self.peer.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("Connected! Connection id: {:?}", conn.connection_id);
                    self.connections.insert(conn);
                }
                GameNetworkEvent::Disconnected(conn) => {
                    println!("Disconnected! Connection id: {:?}", conn.connection_id);
                    if self.connections.remove(&conn) {
                        let player_id = conn.connection_id;
                        let disconnected_payload = serialize_disconnect_payload(&DisconnectPayload { player_id });
                        let topic = Topic::Disconnect(player_id).to_bytes();
                        self.publish(topic, disconnected_payload, self.broker_stream.clone());

                        for subscribers in self.subscriptions.values_mut() {
                            subscribers.remove(&conn);
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
            BrokerMessage::Subscribe { client_id: _, topic } => {
                let topic_desc = Topic::from_bytes(topic);
                println!("Broker: Subscribe - conn_id={:?}, topic={:?}", connection.connection_id, topic_desc);
                self.subscriptions.entry(topic).or_default().insert(connection);
            }
            BrokerMessage::Unsubscribe { client_id: _, topic } => {
                let topic_desc = Topic::from_bytes(topic);
                println!("Broker: Unsubscribe - conn_id={:?}, topic={:?}", connection.connection_id, topic_desc);
                self.subscriptions.entry(topic).or_default().remove(&connection);
            }
            BrokerMessage::Publish { topic, payload } => {
                self.publish(topic, payload, stream);
            }
            BrokerMessage::Connect { client_id: _ } => {
                println!("Broker: Connect from conn_id={:?}", connection.connection_id);
            }
            _ => {}
        }
    }

    fn publish(&self, topic: [u8; 32], payload: Vec<u8>, stream: GameStream) {
        if let Some(subscribers) = self.subscriptions.get(&topic) {
            let broadcast_bytes = BrokerMessage::serialize_broadcast(topic, &payload);
            let bytes_payload = Bytes::from(broadcast_bytes);

            let topic_desc = Topic::from_bytes(topic);
            match topic_desc {
                Topic::Input(_) => {}
                _ => println!("Broker: publishing {:?} to {} subscribers", topic_desc, subscribers.len()),
            }

            for conn in subscribers {
                if let Err(e) = self.peer.send(conn, &stream, bytes_payload.clone()) {
                    eprintln!("Failed to forward publish to {:?}: {:?}", conn.connection_id, e);
                }
            }
        } else {
            println!("Broker: no subscribers for topic {:?}", Topic::from_bytes(topic));
        }
    }
}