use bevy::prelude::*;

use common::broker_messages::BrokerMessage;
use common::topics::{serialize_input_payload, InputPayload, Topic};

use super::{ GameState};
use super::net::{ActivePeer, BrokerConn, BrokerControlStream};

pub struct ClientInputPlugin;

impl Plugin for ClientInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, send_input.run_if(in_state(GameState::InGame)));
    }
}

fn send_input(
    keys: Res<ButtonInput<KeyCode>>,
    peer_res: Option<ResMut<ActivePeer>>,
    broker_conn: Option<Res<BrokerConn>>,
    broker_stream: Option<Res<BrokerControlStream>>,
    mut last_logged_vector: Local<Option<[f64; 2]>>,
) {
    let movement_key_changed = keys.just_pressed(KeyCode::ArrowLeft)
        || keys.just_pressed(KeyCode::KeyA)
        || keys.just_pressed(KeyCode::ArrowRight)
        || keys.just_pressed(KeyCode::KeyD)
        || keys.just_pressed(KeyCode::ArrowUp)
        || keys.just_pressed(KeyCode::KeyW)
        || keys.just_pressed(KeyCode::ArrowDown)
        || keys.just_pressed(KeyCode::KeyS)
        || keys.just_released(KeyCode::ArrowLeft)
        || keys.just_released(KeyCode::KeyA)
        || keys.just_released(KeyCode::ArrowRight)
        || keys.just_released(KeyCode::KeyD)
        || keys.just_released(KeyCode::ArrowUp)
        || keys.just_released(KeyCode::KeyW)
        || keys.just_released(KeyCode::ArrowDown)
        || keys.just_released(KeyCode::KeyS);

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

    if movement_key_changed {
        let current = [dx, dy];
        let should_log = last_logged_vector
            .map(|previous| previous != current)
            .unwrap_or(true);
        if should_log {
            tracing::info!("Movement input detected: dx={dx}, dy={dy}");
            *last_logged_vector = Some(current);
        }
    }

    let (Some(peer_res), Some(broker_conn), Some(broker_stream)) = (peer_res, broker_conn, broker_stream) else {
        if movement_key_changed {
            tracing::warn!("Movement keys changed but ActivePeer/BrokerConn/BrokerControlStream is missing; input cannot be sent");
        }
        return;
    };
    let Ok(peer) = peer_res.0.lock() else { return };

    // Only send when a key is held — avoids spamming zero-input datagrams.
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    
    let payload = serialize_input_payload(&InputPayload {
        dxdy: [dx, dy],
    });

    let topic = Topic::Input(broker_conn.0.connection_id).to_bytes();
    let publish = BrokerMessage::serialize_publish(topic, &payload);
    if let Err(e) = peer.send(&broker_conn.0, &broker_stream.0, publish.into()) {
        tracing::warn!("send (input publish): {e:?}");
    } else if movement_key_changed {
        tracing::info!(
            "Sent input payload: dx={dx}, dy={dy} for connection_id={}",
            broker_conn.0.connection_id
        );
    }
}
