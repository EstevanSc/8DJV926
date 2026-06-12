use bevy::{prelude::*};

use common::broker_messages::BrokerMessage;
use common::topics::{serialize_input_payload, InputPayload, Topic};

use super::{ GameState};
use super::net::{ActivePeer, BrokerConn, BrokerControlStream};

use crate::src::interpolation::RemotePlayer;
use crate::src::interpolation::SelfPlayer;

pub struct ClientInputPlugin;

impl Plugin for ClientInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, keyboard_input.run_if(in_state(GameState::InGame)))
            .add_systems(Update, mouse_button_input.run_if(in_state(GameState::InGame)))
            .init_resource::<PathToCursor>();
    }
}

fn send_input(

    peer_res: Option<ResMut<ActivePeer>>,
    broker_conn: Option<Res<BrokerConn>>,
    broker_stream: Option<Res<BrokerControlStream>>,
    input: [f64; 2]
) {
    let (Some(peer_res), Some(broker_conn), Some(broker_stream)) = (peer_res, broker_conn, broker_stream) else {
        return;
    };
    let Ok(peer) = peer_res.0.lock() else { return };

    // Only send when a key is held — avoids spamming zero-input datagrams.
    let [dx, dy] = input;
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
    }
}

fn keyboard_input(keys: Res<ButtonInput<KeyCode>>,
    peer_res: Option<ResMut<ActivePeer>>,
    broker_conn: Option<Res<BrokerConn>>,
    broker_stream: Option<Res<BrokerControlStream>>,
    mut path_to_cursor: ResMut<PathToCursor>,
) {

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

    if dx == 0.0 && dy == 0.0 {
        return;
    }

    path_to_cursor.path.clear();

    send_input(peer_res, broker_conn, broker_stream, [dx, dy]);
}

#[derive(Resource, Reflect, Default)]
struct PathToCursor{
    path: Vec<Vec2>
}

fn mouse_button_input(
    peer_res: Option<ResMut<ActivePeer>>,
    broker_conn: Option<Res<BrokerConn>>,
    broker_stream: Option<Res<BrokerControlStream>>,
    buttons: Res<ButtonInput<MouseButton>>,
    query_player: Query<(Entity, &RemotePlayer, &Transform, &SelfPlayer)>,
    q_window: Query<&Window>,
    q_camera: Query<(&Camera, &GlobalTransform)>,
    mut path_to_cursor: ResMut<PathToCursor>
) {
    if buttons.just_pressed(MouseButton::Right) {
        path_to_cursor.path.clear();

        let cursor_world_position = get_cursor_world_position(q_window, q_camera);
        info!("Cursor world position: {:?}", cursor_world_position);
        path_to_cursor.path.push(cursor_world_position);
    }

    if path_to_cursor.path.len() >= 1 {
        let target = path_to_cursor.path[0];
        let player_position = query_player.single().map(|(_, _, transform, _)| transform.translation).unwrap_or_default();

        if player_position.truncate().distance(target) < 10.0 {
            path_to_cursor.path.remove(0);
            return;
        }

        let direction = (target - player_position.truncate()).normalize_or_zero();
        send_input(peer_res, broker_conn, broker_stream, [direction.x as f64, direction.y as f64]);
    }
}

fn get_cursor_world_position(
    // Query the primary window to get the cursor position
    q_window: Query<&Window>,
    // Query the camera transform and projection
    q_camera: Query<(&Camera, &GlobalTransform)>,
) -> Vec2 {
    // Get the primary window
    let window = q_window.single().unwrap();
    
    // Get the camera and its global transform
    let (camera, camera_transform) = q_camera.single().unwrap();

    // 1. Check if the cursor is inside the window and get its position
    if let Some(cursor_screen_pos) = window.cursor_position() {
        
        // 2. Convert the screen position to world coordinates (Now returns a Result!)
        if let Ok(cursor_world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen_pos) {
            
            // cursor_world_pos is a Vec2 containing the exact world coordinates
            info!("Cursor World Position: X: {}, Y: {}", cursor_world_pos.x, cursor_world_pos.y);
            return cursor_world_pos;
        }
    }
    Vec2::ZERO
}