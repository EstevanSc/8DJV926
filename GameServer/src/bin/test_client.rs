use game_sockets::protocols::QuicBackend;
use game_sockets::{GameNetworkEvent, GamePeer, GameStream};
use serde::{Deserialize, Serialize};
use std::thread;
use std::time::Duration;
use uuid::Uuid;
use wincode::{SchemaRead, SchemaWrite};

// Replicating the exact message format your server expects
#[derive(Debug, Serialize, Deserialize, Clone, SchemaWrite, SchemaRead)]
pub enum GameMessage {
    Join { username: String },
    Welcome { player_id: Uuid },
}

fn main() {
    println!("Connecting to GameServer...");

    // 1. Initialize the networking client
    let mut client = GamePeer::new(QuicBackend::new());

    // Server endpoint configuration (Adjust if running via Docker)
    let server_ip = "127.0.0.1";
    let server_port = 7777;

    // 2. Establish a connection to the Bevy Dedicated Server
    match client.connect(server_ip, server_port) {
        Ok(conn) => {
            println!(
                "QUIC connection handshake initiated with {}:{}",
                server_ip, server_port
            );
            conn
        }
        Err(e) => {
            eprintln!("Failed to connect to server: {:?}", e);
            return;
        }
    };

    // 3. Start the polling network event loop
    loop {
        match client.poll() {
            Ok(Some(event)) => {
                match event {
                    GameNetworkEvent::Connected(conn) => {
                        println!(
                            "Successfully connected! Connection ID: {:?}",
                            conn.connection_id
                        );

                        // 4. Send the GameMessage::Join packet once connected
                        let join_msg = GameMessage::Join {
                            username: "Alice_Tester".to_string(),
                        };

                        if let Ok(serialized) = wincode::serialize(&join_msg) {
                            let stream = GameStream::from(0);
                            if let Err(e) = client.send(&conn, &stream, serialized.into()) {
                                eprintln!("Failed to send JOIN message: {:?}", e);
                            } else {
                                println!("Sent JOIN message for user 'Alice_Tester'");
                            }
                        } else {
                            eprintln!("Failed to serialize game message");
                        }
                    }

                    GameNetworkEvent::Message {
                        data,
                        connection: _,
                        stream: _,
                    } => {
                        // 5. Catch and deserialize the incoming welcome message from the server
                        match wincode::deserialize::<GameMessage>(&data) {
                            Ok(GameMessage::Welcome { player_id }) => {
                                println!("Received WELCOME from server.");
                                println!("Assigned Player UUID: {}", player_id);
                                println!("Disconnecting cleanly in 3 seconds...");

                                thread::sleep(Duration::from_secs(3));

                                break;
                            }
                            Ok(other) => {
                                println!("Received unexpected message variant: {:?}", other)
                            }
                            Err(e) => eprintln!("Failed to parse incoming packet: {:?}", e),
                        }
                    }

                    GameNetworkEvent::Disconnected(_) => {
                        println!("Disconnected from server.");
                        break;
                    }
                    _ => {}
                }
            }
            Ok(None) => {
                // Throttle CPU usage slightly while idling for packets
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                eprintln!("Network polling error occurred: {:?}", e);
                break;
            }
        }
    }

    println!("Shutting down network peer cleanly...");
    if let Err(e) = client.shutdown() {
        eprintln!("Failed to shut down client cleanly: {:?}", e);
    } else {
        println!("Client disconnected successfully.");
    }
}
