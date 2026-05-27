use crate::net::{BrokerConfig, BrokerState};

mod net;
mod messages;

fn main() {
    let config = BrokerConfig::from_env();
    if let Ok(peer) = net::bind_socket(&config) {
        let mut broker = BrokerState::new(peer);

        println!("Broker successfully started on port {}", config.port);

        loop {
            broker.receive_packets();
        }
    }
}
