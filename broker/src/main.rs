use crate::net::{BrokerConfig, BrokerState};
use std::collections::HashMap;
mod net;

fn main() {
    let config = BrokerConfig::from_env();
    if let Ok(peer) = net::bind_socket(&config) {
        let mut broker = BrokerState::new(peer);

        println!("Broker successfully started on port {}", config.port);

        let mut connection_map = HashMap::new();

        loop {
            broker.receive_packets(&mut connection_map);
        }
    }
}
