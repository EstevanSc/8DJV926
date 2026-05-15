use std::time::Duration;
use bytes::Bytes;
use game_sockets;
use game_sockets::{GameConnection, GameNetworkEvent, GamePeer, GameStream, GameStreamReliability};
use game_sockets::protocols::QuicBackend;

fn main() {
    let mut server = GamePeer::new(QuicBackend::new());
    server.listen("127.0.0.1", 9876).expect("Server failed to bind");
    println!("[Server] Listening on 9876...");

    let mut client = GamePeer::new(QuicBackend::new());
    client.connect("127.0.0.1", 9876).expect("Client failed to connect");
    println!("[Client] Connecting...");

    let mut client_conn = None;
    let mut server_conn = None;
    let mut stream_ready = false;

    for frame in 0..100 {
        // CLIENT
        while let Ok(Some(event)) = client.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("[Client] Connected! ID: {:?}", conn.connection_id);
                    client_conn = Some(conn);
                    client.create_stream(conn, GameStreamReliability::Reliable).unwrap();
                }
                GameNetworkEvent::StreamCreated(_, _) => {
                    println!("[Client] Reliable Stream Established.");
                    stream_ready = true;
                }
                GameNetworkEvent::Message { data, .. } => {
                    println!("[Client] Received: {:?}", String::from_utf8_lossy(&data));
                }
                _ => {}
            }
        }

        // SERVER
        while let Ok(Some(event)) = server.poll() {
            match event {
                GameNetworkEvent::Connected(conn) => {
                    println!("[Server] New Client Connected: {:?}", conn.connection_id);
                    server_conn = Some(conn);
                }
                GameNetworkEvent::Message { data, connection, stream } => {
                    let msg = String::from_utf8_lossy(&data);
                    println!("[Server] Received: '{}'. Echoing back!", msg);
                    server.send(&connection, &stream, data).unwrap();
                }
                _ => {}
            }
        }

        // Send data every 20 frame
        if stream_ready && frame % 20 == 0 {
            if let (Some(conn), true) = (client_conn, stream_ready) {
                let msg = Bytes::from(format!("Hello from frame {}", frame));
                let stream = GameStream::new(1, GameStreamReliability::Reliable);
                client.send(&conn, &stream, msg).unwrap();
            }
        }

        // 60fps
        std::thread::sleep(Duration::from_millis(16));
    }

    println!("\nGoodbye");
    server.shutdown().unwrap();
    client.shutdown().unwrap();
}
