use bevy::{ prelude::*};

use common::constants::POSITION_DELTA_THRESHOLD;

use super::net::{BrokerConn, PositionUpdateReceived, QuadtreeBoundariesUpdateReceived};
use super::{GameSession, GameState};

pub struct InterpolationPlugin;

impl Plugin for InterpolationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::InGame), (spawn_floor, spawn_debug_hud))
            .add_systems(
                Update,
                (spawn_remote_players, interpolate_remote_players, update_remote_player_labels, follow_local_player, draw_debug_quad_tree)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Tracks a remote (server-side) entity on the client.
#[derive(Component)]
pub struct RemotePlayer {
    pub connection_id: uuid::Uuid,
    pub target: Vec2,
    pub prev: Vec2,
}

/// Marks the text label attached to a remote player.
#[derive(Component)]
pub struct RemotePlayerLabel {
    pub connection_id: uuid::Uuid,
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Marker for entities that belong to the in-game scene.
#[derive(Component)]
pub struct GameSceneRoot;

#[derive(Component)]
struct FollowCamera;


#[derive(Component)]
struct DebugQuadTree;

/// Spawn a camera and a visual floor mesh when entering InGame.
fn spawn_floor(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn((Camera2d, FollowCamera, GameSceneRoot));
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(4000.0, 32.0))),
        MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::srgb(0.25, 0.22, 0.18)))),
        Transform::from_translation(Vec3::new(0.0, -300.0, 0.0)),
        GameSceneRoot,
    ));
}

fn draw_debug_quad_tree(mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut events: MessageReader<QuadtreeBoundariesUpdateReceived>,
    query: Query<Entity, With<DebugQuadTree>>,
) {
    let color_array = [Color::srgba(1.0, 0.0, 0.0, 0.125), Color::srgba(0.0, 1.0, 0.0, 0.125), Color::srgba(0.0, 0.0, 1.0, 0.125), Color::srgba(1.0, 1.0, 0.0, 0.125)];

    let margin_color_array  = [Color::srgba(1.0, 0.0, 0.0, 0.125), Color::srgba(0.0, 1.0, 0.0, 0.125), Color::srgba(0.0, 0.0, 1.0, 0.125), Color::srgba(1.0, 1.0, 0.0, 0.125)];
    for update in events.read() {
        for entity in query.iter() {
            commands.entity(entity).despawn();
        }
        let margin = update.payload.margin;
        for (i, boundary) in update.payload.boundaries.iter().enumerate() {
            let center = Vec2::new(boundary.x as f32, boundary.y as f32);
            let size = Vec2::new((boundary.half_size * 2.0 - margin as f64 * 2.0) as f32, (boundary.half_size * 2.0 - margin as f64 * 2.0) as f32);
            let color = color_array[i % color_array.len()];
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(size.x, size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(color))),
                Transform::from_translation(center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree
            ));

            let margin_color = margin_color_array[i % margin_color_array.len()];
            //top margin
            let outer_size = Vec2::new(size.x + margin * 2.0, margin);
            let top_center = Vec2::new(center.x, center.y + boundary.half_size as f32 + margin / 2.0);
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(top_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree
            ));
            //bottom margin
            let bottom_center = Vec2::new(center.x, center.y - boundary.half_size as f32 - margin / 2.0);
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(bottom_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree
            ));
            //left margin
            let outer_size = Vec2::new(margin, size.y + margin * 2.0);
            let left_center = Vec2::new(center.x - boundary.half_size as f32 - margin / 2.0, center.y);
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(left_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree
            ));
            //right margin
            let right_center = Vec2::new(center.x + boundary.half_size as f32 + margin / 2.0, center.y);
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(right_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree
            ));
        }
    }
}


fn follow_local_player(
    broker_conn: Option<Res<BrokerConn>>,
    player_query: Query<(&RemotePlayer, &Transform), Without<FollowCamera>>,
    mut camera_query: Query<&mut Transform, (With<Camera>, With<FollowCamera>, Without<RemotePlayer>)>,
) {
    let Some(broker_conn) = broker_conn else { return };
    let my_id = broker_conn.0.connection_id;

    let Some((_, player_transform)) = player_query
        .iter()
        .find(|(player, _)| player.connection_id == my_id)
    else {
        return;
    };

    let Ok(mut camera_transform) = camera_query.single_mut() else { return };

    let target = Vec3::new(
        player_transform.translation.x,
        player_transform.translation.y,
        camera_transform.translation.z,
    );

    camera_transform.translation = target;
}

/// Spawn a circle for each new entity_id seen in position batches.
/// Own player is rendered green; other players are blue.
/// A name tag is spawned as a child entity above each circle.
fn spawn_remote_players(
    mut commands: Commands,
    mut events: MessageReader<PositionUpdateReceived>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    existing: Query<&RemotePlayer>,
    broker_conn: Option<Res<BrokerConn>>,
) {
    let my_connection_id = broker_conn.map(|r| r.0.connection_id);
    for update in events.read() {
        let connection_id = update.connection_id;
        let already_exists = existing.iter().any(|r| r.connection_id == connection_id);
        if !already_exists {
            let pos = Vec2::new(update.payload.position[0] as f32, update.payload.position[1] as f32);
            let is_me = my_connection_id == Some(connection_id);
            let color = if is_me {
                Color::srgb(0.2, 1.0, 0.2) // green = local player
            } else {
                Color::srgb(0.2, 0.6, 1.0) // blue = other players
            };
            let name = format!("Entity {}", connection_id);
            commands.spawn((
                RemotePlayer {
                    connection_id: update.connection_id,
                    target: pos,
                    prev: pos,
                },
                Mesh2d(meshes.add(Circle::new(16.0))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(color))),
                Transform::from_translation(pos.extend(0.0)),
            )).with_children(|parent| {
                let label_text = format_remote_player_label(&name, pos);
                parent.spawn((
                    Text2d::new(label_text),
                    TextFont { font_size: 12.0, ..default() },
                    TextColor(Color::WHITE),
                    Transform::from_translation(Vec3::new(0.0, 28.0, 1.0)),
                    RemotePlayerLabel {
                        connection_id: update.connection_id,
                        display_name: name,
                    },
                ));
            });
        }
    }
}

/// Smooth-step each remote player toward its latest received position.
fn interpolate_remote_players(
    mut events: MessageReader<PositionUpdateReceived>,
    mut query: Query<(&mut RemotePlayer, &mut Transform)>,
    time: Res<Time>,
) {
    // Apply latest update target for each incoming entity position.
    for update in events.read() {
        for (mut remote, _) in &mut query {
            if remote.connection_id == update.connection_id {
                let new_pos = Vec2::new(update.payload.position[0] as f32, update.payload.position[1] as f32);
                if (new_pos - remote.target).length() > POSITION_DELTA_THRESHOLD {
                    remote.prev = remote.target;
                    remote.target = new_pos;
                }
            }
        }
    }

    // Lerp toward target every frame.
    let alpha = (time.delta_secs() * 15.0).min(1.0);
    for (remote, mut transform) in &mut query {
        let current = transform.translation.truncate();
        let next = current.lerp(remote.target, alpha);
        transform.translation = next.extend(transform.translation.z);
    }
}

/// Update the text shown under each remote player to include its rounded position.
fn update_remote_player_labels(
    query: Query<(&RemotePlayer, &Transform, &Children)>,
    mut labels: Query<(&RemotePlayerLabel, &mut Text2d)>,
) {
    for (remote, transform, children) in &query {
        let position = transform.translation.truncate();
        for child in children.iter() {
            if let Ok((tag, mut text)) = labels.get_mut(child) {
                if tag.connection_id == remote.connection_id {
                    text.0 = format_remote_player_label(&tag.display_name, position);
                }
            }
        }
    }
}

fn format_remote_player_label(name: &str, position: Vec2) -> String {
    format!("{}\n({:03.0}, {:03.0})", name, position.x, position.y)
}

/// Spawn a top-left debug overlay showing session info. Cleared with the rest
/// of the game scene when leaving InGame (all entities have GameSceneRoot).
fn spawn_debug_hud(
    mut commands: Commands,
    session: Res<GameSession>,
    broker_conn: Option<Res<BrokerConn>>,
) {
    let connection_id = broker_conn
        .map(|r| r.0.connection_id.to_string())
        .unwrap_or_else(|| "—".to_string());

    let info = format!(
        "Player    : {}\nConnection ID : {}",
        session.username,
        //session.player_id,
        connection_id,
        /*session.server_ip,
        session.server_port,
        session.server_zone,*/
    );

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(8.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            GameSceneRoot,
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(info),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));
        });
}

