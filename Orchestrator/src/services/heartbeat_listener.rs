//! UDP heartbeat listener for server status updates.

use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Starts the UDP heartbeat listener on the configured port.
/// Continuously listens for incoming heartbeat packets from Dedicated Servers
/// and logs received payloads.
pub async fn start_heartbeat_listener(port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    match UdpSocket::bind(addr).await {
        Ok(socket) => {
            tracing::info!("Heartbeat listener started on {}", addr);
            listen_loop(socket).await;
        }
        Err(e) => {
            tracing::error!("Failed to bind heartbeat listener on {}: {}", addr, e);
        }
    }
}

/// Continuously receives UDP packets and logs them.
async fn listen_loop(socket: UdpSocket) {
    let mut buffer = vec![0u8; 65535];

    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((n, addr)) => {
                let payload = &buffer[..n];
                tracing::info!("Received heartbeat from {}: {} bytes", addr, n);
                tracing::debug!("Payload: {:?}", String::from_utf8_lossy(payload));
            }
            Err(e) => {
                tracing::error!("Error receiving heartbeat: {}", e);
            }
        }
    }
}
