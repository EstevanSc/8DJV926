use bevy::prelude::*;

use common::packets::PlayerInput;

use super::GameState;
use super::net::{ActivePeer, ServerConn};

pub struct ClientInputPlugin;

impl Plugin for ClientInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, send_input.run_if(in_state(GameState::InGame)));
    }
}

fn send_input(
    keys: Res<ButtonInput<KeyCode>>,
    peer_res: Option<ResMut<ActivePeer>>,
    server_conn: Option<Res<ServerConn>>,
) {
    let (Some(peer_res), Some(server_conn)) = (peer_res, server_conn) else {
        return;
    };
    let Ok(peer) = peer_res.0.lock() else { return };

    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    if keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA) {
        dx -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD) {
        dx += 1.0;
    }
    if keys.pressed(KeyCode::ArrowUp) || keys.pressed(KeyCode::KeyW) {
        dy += 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) || keys.pressed(KeyCode::KeyS) {
        dy -= 1.0;
    }

    // Only send when a key is held — avoids spamming zero-input datagrams.
    if dx == 0.0 && dy == 0.0 {
        return;
    }

    let data = wincode::serialize(&PlayerInput { dx, dy })
        .expect("failed to serialize PlayerInput");
    let stream = game_sockets::GameStream::from(0);
    if let Err(e) = peer.send(&server_conn.0, &stream, data.into()) {
        tracing::warn!("send (input): {e:?}");
    }
}
