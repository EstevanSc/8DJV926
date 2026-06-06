use std::collections::{HashMap, HashSet};
use bytes::Bytes;
use game_sockets;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameSocketError, GameStream, GameStreamReliability};
use game_sockets::protocols::QuicBackend;
use common::broker_messages::{BrokerMessage};
use common::topics::Topic;

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
    pub fn receive_packets(&mut self, connection_map: &mut HashMap<uuid::Uuid, GameConnection>) {
        while let Ok(Some(event)) = self.peer.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("Connected! Connection id: {:?}", conn.connection_id);
                    self.connections.insert(conn);
                    connection_map.insert(conn.connection_id, conn);
                }
                GameNetworkEvent::Disconnected(conn) => {
                    println!("Disconnected! Connection id: {:?}", conn.connection_id);
                    if self.connections.remove(&conn) {
                        let connection_id = conn.connection_id;
                        let topic = Topic::Disconnect(connection_id).to_bytes();
                        self.publish(topic, Vec::new());

                        for subscribers in self.subscriptions.values_mut() {
                            subscribers.remove(&conn);
                        }
                        connection_map.remove(&connection_id);
                    }
                }
                GameNetworkEvent::Message {
                    data,
                    connection,
                    stream: _,
                } => {
                    self.handle_message(data, connection, connection_map);
                }
                _ => {}
            }
        }
    }

    fn handle_message(&mut self, data: Bytes, connection: GameConnection, connection_map: &mut HashMap<uuid::Uuid, GameConnection>) {
        let message = match BrokerMessage::deserialize(&data) {
            Some(msg) => msg,
            None => {
                eprintln!("Received malformed or unknown packet tag from {:?}", connection.connection_id);
                return;
            }
        };

        match message {
            BrokerMessage::Subscribe { client_id, topic } => {
                let topic_desc = Topic::from_bytes(topic); 
                println!("Broker: Subscribe - conn_id={:?}, topic={:?}", client_id, topic_desc);
                if let Some(existing_connection) = connection_map.get(&client_id) {
                    self.subscriptions.entry(topic).or_default().insert(*existing_connection);
                }
            }
            BrokerMessage::Unsubscribe { client_id, topic } => {
                let topic_desc = Topic::from_bytes(topic);
                println!("Broker: Unsubscribe - conn_id={:?}, topic={:?}", client_id, topic_desc);
                if let Some(existing_connection) = connection_map.get(&client_id) {
                    self.subscriptions.entry(topic).or_default().remove(existing_connection);  
                }
            }
            BrokerMessage::Publish { topic, payload } => {
                self.publish(topic, payload);
            }
            BrokerMessage::Connect { client_id, sending_system } => {
                println!("Broker: Connect from conn_id={:?}, sending_system={:?}", client_id, sending_system);
                connection_map.insert(client_id, connection);
            }
            _ => {}
        }
    }

    fn publish(&self, topic: [u8; 32], payload: Vec<u8>) {
        if let Some(subscribers) = self.subscriptions.get(&topic) {
            let broadcast_bytes = BrokerMessage::serialize_broadcast(topic, &payload);
            let bytes_payload = Bytes::from(broadcast_bytes);

            let topic_desc = Topic::from_bytes(topic);
            match topic_desc {
                Topic::Input(_) => {}
                Topic::EntityPositionUpdate(_) => {}
                Topic::AuthorityDebugPacket(_) => {}
                _ => println!("Broker: publishing {:?} to {} subscribers. The ids of the subscribers are {:?}", topic_desc, subscribers.len(), subscribers.iter().map(|c| c.connection_id).collect::<Vec<_>>()),
            }

            for conn in subscribers {
                if let Err(e) = self.peer.send(conn, &self.broker_stream, bytes_payload.clone()) {
                    eprintln!("Failed to forward publish to {:?}: {:?}", conn.connection_id, e);
                }
            }
        } else {
            println!("Broker: no subscribers for topic {:?}", Topic::from_bytes(topic));
        }
    }
}