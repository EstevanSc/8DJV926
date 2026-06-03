use bevy::prelude::*;

use common::broker_messages::BrokerMessage;
use common::topics::{serialize_input_payload, InputPayload, Topic};
use uuid::Uuid;

use super::{GameSession, GameState};
use super::net::{ActivePeer, BrokerConn};

pub struct ClientInputPlugin;

impl Plugin for ClientInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, send_input.run_if(in_state(GameState::InGame)));
    }
}

fn send_input(
    keys: Res<ButtonInput<KeyCode>>,
    session: Res<GameSession>,
    peer_res: Option<ResMut<ActivePeer>>,
    broker_conn: Option<Res<BrokerConn>>,
) {
    let (Some(peer_res), Some(broker_conn)) = (peer_res, broker_conn) else {
        return;
    };
    let Ok(peer) = peer_res.0.lock() else { return };

    let mut dx = 0.0_f64;
    let mut dy = 0.0_f64;
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

    let Ok(player_id) = Uuid::parse_str(&session.player_id) else {
        tracing::warn!("send (input): invalid player_id '{}'; skipping", session.player_id);
        return;
    };

    let payload = serialize_input_payload(&InputPayload {
        player_id,
        dxdy: [dx, dy],
    });

    let topic = Topic::Input(player_id).to_bytes();
    let publish = BrokerMessage::serialize_publish(topic, &payload);
    let stream = game_sockets::GameStream::from(0);
    if let Err(e) = peer.send(&broker_conn.0, &stream, publish.into()) {
        tracing::warn!("send (input publish): {e:?}");
    }
}
