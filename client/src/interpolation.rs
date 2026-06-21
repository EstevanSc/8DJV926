use bevy::prelude::*;

use common::constants::POSITION_DELTA_THRESHOLD;
use common::map_data::{BitMap, MAP_HEIGHT, MAP_WIDTH, TILE_SIZE};
use common::topics::Topic;

use super::net::{
    ActivePeer, AttributeUpdatedReceived, AuthorityDebugPacketReceived, BrokerConn,
    BrokerControlStream, DisconnectReceived, PositionUpdateReceived,
    QuadtreeBoundariesUpdateReceived, LevelUpReceived, XPEarnedReceived,
};
use super::{GameSession, GameState};

pub struct InterpolationPlugin;

impl Plugin for InterpolationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DiscoveredEntities>()
            .add_systems(OnEnter(GameState::InGame), (spawn_map, spawn_debug_hud))
            .add_systems(
                Update,
                (
                    handle_name_responses,
                    spawn_remote_players,
                    interpolate_remote_players,
                    update_remote_player_labels,
                    follow_local_player,
                    draw_debug_quad_tree,
                    spawn_debug_hud,
                    handle_disconnect,
                    delete_entities_outside_of_interest,
                    spawn_remote_fireballs,
                    update_projectiles,
                    handle_attribute_updates,
                    handle_xp_and_level_updates,
                    update_player_stats_labels,
                )
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

#[derive(Component)]
pub struct ClientProjectile {
    pub direction: Vec2,
    pub speed: f32,
}

#[derive(Component)]
pub struct PlayerStats {
    pub hp: i32,
    pub mp: i32,
    pub level: i32,
    pub xp: i32,
}

#[derive(Component)]
pub struct RemotePlayerStatsLabel {
    pub connection_id: uuid::Uuid,
}

#[derive(Resource, Default)]
pub struct DiscoveredEntities(pub std::collections::HashMap<uuid::Uuid, String>);

fn handle_name_responses(
    mut events: MessageReader<super::net::DbNameResponseReceived>,
    mut discovered: ResMut<DiscoveredEntities>,
) {
    for ev in events.read() {
        discovered.0.insert(ev.player_id, ev.username.clone());
        tracing::info!("Resolved player name: {} -> {}", ev.player_id, ev.username);
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Marker for entities that belong to the in-game scene.
#[derive(Component)]
pub struct GameSceneRoot;

#[derive(Component)]
pub struct FollowCamera;

#[derive(Component)]
struct DebugQuadTree;

#[derive(Component)]
struct DebugUI;

#[derive(Component)]
pub struct SelfPlayer;

/// Spawn a camera and a visual floor mesh when entering InGame.
fn spawn_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let mut map = BitMap::new();
    map.generate_map();
    map.print_sub_grid(0, 0, 64, 32);

    for y in 0..map.data.len() {
        for x in 0..map.data[y].len() * 64 {
            if map.is_wall(x, y) {
                // 1. Convert tile coordinate to initial world space
                // 2. Subtract half-map size to center it at (0,0)
                // 3. Add 4.0 (half of tile size) so the anchor aligns to the center of the asset mesh
                let world_x = (x as f32 * TILE_SIZE) - (MAP_WIDTH as f32 * TILE_SIZE / 2.0)
                    + (TILE_SIZE / 2.0);
                let world_y = (y as f32 * TILE_SIZE) - (MAP_HEIGHT as f32 * TILE_SIZE / 2.0)
                    + (TILE_SIZE / 2.0);

                commands.spawn((
                    Mesh2d(meshes.add(Rectangle::new(TILE_SIZE, TILE_SIZE))),
                    MeshMaterial2d(
                        materials.add(ColorMaterial::from_color(Color::srgb(0.0, 0.0, 0.0))),
                    ),
                    Transform::from_translation(Vec3::new(world_x, world_y, 1.0)),
                    GameSceneRoot,
                ));
            }
        }
    }

    commands.spawn((Camera2d, FollowCamera, GameSceneRoot));
}

fn draw_debug_quad_tree(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut events: MessageReader<QuadtreeBoundariesUpdateReceived>,
    query: Query<Entity, With<DebugQuadTree>>,
) {
    let color_array = [
        Color::srgba(1.0, 0.0, 0.0, 0.125), // 1. Red (Primary)
        Color::srgba(1.0, 0.5, 0.0, 0.125), // 2. Orange (Tertiary)
        Color::srgba(1.0, 1.0, 0.0, 0.125), // 3. Yellow (Secondary)
        Color::srgba(0.5, 1.0, 0.0, 0.125), // 4. Lime / Chartreuse (Tertiary)
        Color::srgba(0.0, 1.0, 0.0, 0.125), // 5. Green (Primary)
        Color::srgba(0.0, 1.0, 0.5, 0.125), // 6. Spring Green (Tertiary)
        Color::srgba(0.0, 1.0, 1.0, 0.125), // 7. Cyan / Aqua (Secondary)
        Color::srgba(0.0, 0.5, 1.0, 0.125), // 8. Azure / Sky Blue (Tertiary)
        Color::srgba(0.0, 0.0, 1.0, 0.125), // 9. Blue (Primary)
        Color::srgba(0.5, 0.0, 1.0, 0.125), // 10. Violet / Purple (Tertiary)
        Color::srgba(1.0, 0.0, 1.0, 0.125), // 11. Magenta / Fuchsia (Secondary)
        Color::srgba(1.0, 0.0, 0.5, 0.125), // 12. Rose (Tertiary)
    ];

    //let margin_color_array  = [Color::srgba(1.0, 0.0, 0.0, 0.125), Color::srgba(0.0, 1.0, 0.0, 0.125), Color::srgba(0.0, 0.0, 1.0, 0.125), Color::srgba(1.0, 1.0, 0.0, 0.125)];
    for update in events.read() {
        for entity in query.iter() {
            commands.entity(entity).despawn();
        }
        let margin = update.payload.margin;
        for (i, boundary) in update.payload.boundaries.iter().enumerate() {
            let center = Vec2::new(boundary.x as f32, boundary.y as f32);
            let size = Vec2::new(
                (boundary.half_size * 2.0 - margin as f64 * 2.0) as f32,
                (boundary.half_size * 2.0 - margin as f64 * 2.0) as f32,
            );
            let color = color_array[i % color_array.len()];
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(size.x, size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(color))),
                Transform::from_translation(center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree,
            ));

            let margin_color = color_array[i % color_array.len()];
            //top margin
            let outer_size = Vec2::new(size.x + margin * 2.0, margin);
            let top_center = Vec2::new(
                center.x,
                center.y + boundary.half_size as f32 + margin / 2.0,
            );
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(top_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree,
            ));
            //bottom margin
            let bottom_center = Vec2::new(
                center.x,
                center.y - boundary.half_size as f32 - margin / 2.0,
            );
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(bottom_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree,
            ));
            //left margin
            let outer_size = Vec2::new(margin, size.y + margin * 2.0);
            let left_center = Vec2::new(
                center.x - boundary.half_size as f32 - margin / 2.0,
                center.y,
            );
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(left_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree,
            ));
            //right margin
            let right_center = Vec2::new(
                center.x + boundary.half_size as f32 + margin / 2.0,
                center.y,
            );
            commands.spawn((
                Mesh2d(meshes.add(Rectangle::new(outer_size.x, outer_size.y))),
                MeshMaterial2d(materials.add(ColorMaterial::from_color(margin_color))),
                Transform::from_translation(right_center.extend(0.0)),
                GameSceneRoot,
                DebugQuadTree,
            ));
        }
    }
}

fn follow_local_player(
    broker_conn: Option<Res<BrokerConn>>,
    player_query: Query<(&RemotePlayer, &Transform), Without<FollowCamera>>,
    mut camera_query: Query<
        &mut Transform,
        (With<Camera>, With<FollowCamera>, Without<RemotePlayer>),
    >,
) {
    let Some(broker_conn) = broker_conn else {
        return;
    };
    let my_id = broker_conn.0.connection_id;

    let Some((_, player_transform)) = player_query
        .iter()
        .find(|(player, _)| player.connection_id == my_id)
    else {
        return;
    };

    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };

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
    mut peer_res: Option<ResMut<ActivePeer>>,
    broker_stream: Option<Res<BrokerControlStream>>,
    mut discovered: ResMut<DiscoveredEntities>,
    session: Res<GameSession>,
) {
    let my_connection_id = broker_conn.as_ref().map(|r| r.0.connection_id);
    let mut spawned_this_frame = std::collections::HashSet::new();
    for update in events.read() {
        let connection_id = update.connection_id;
        let already_exists = existing.iter().any(|r| r.connection_id == connection_id)
            || spawned_this_frame.contains(&connection_id);
        if !already_exists {
            spawned_this_frame.insert(connection_id);
            let pos = Vec2::new(
                update.payload.position[0] as f32,
                update.payload.position[1] as f32,
            );
            let is_me = my_connection_id == Some(connection_id);
            let color = if is_me {
                Color::srgb(0.2, 1.0, 0.2) // green = local player
            } else {
                Color::srgb(0.2, 0.6, 1.0) // blue = other players
            };

            // Get or request name
            let name = if is_me {
                let my_name = session.username.clone();
                discovered.0.insert(connection_id, my_name.clone());
                my_name
            } else {
                if !discovered.0.contains_key(&connection_id) {
                    // Send DbNameRequest to database_service via broker
                    if let Some(my_id) = my_connection_id {
                        let request_payload = common::topics::serialize_db_name_request_payload(
                            &common::topics::DbNameRequestPayload {
                                requestor_id: my_id,
                                player_id: connection_id,
                            },
                        );
                        if let (Some(peer_res), Some(conn), Some(stream)) = (
                            peer_res.as_mut(),
                            broker_conn.as_ref(),
                            broker_stream.as_ref(),
                        ) {
                            if let Ok(peer) = peer_res.0.lock() {
                                // First subscribe to the topic for the resolved player's name response
                                let subscribe_name =
                                    common::broker_messages::BrokerMessage::serialize_subscribe(
                                        conn.0.connection_id,
                                        Topic::DbNameResponse(connection_id).to_bytes(),
                                    );
                                if let Err(e) = peer.send(&conn.0, &stream.0, subscribe_name.into())
                                {
                                    tracing::error!(
                                        "Failed to subscribe to DbNameResponse for player {}: {:?}",
                                        connection_id,
                                        e
                                    );
                                } else {
                                    tracing::info!(
                                        "Subscribed to DbNameResponse for player: {}",
                                        connection_id
                                    );
                                }



                                // Then publish the name request
                                let publish =
                                    common::broker_messages::BrokerMessage::serialize_publish(
                                        Topic::DbNameRequest.to_bytes(),
                                        &request_payload,
                                    );
                                if let Err(e) = peer.send(&conn.0, &stream.0, publish.into()) {
                                    tracing::warn!("Failed to send DbNameRequest: {:?}", e);
                                } else {
                                    tracing::info!(
                                        "Sent DbNameRequest for remote player: {}",
                                        connection_id
                                    );
                                }
                            }
                        }
                    }
                    let placeholder = format!("Loading...");
                    discovered.0.insert(connection_id, placeholder.clone());
                    placeholder
                } else {
                    discovered
                        .0
                        .get(&connection_id)
                        .cloned()
                        .unwrap_or_else(|| "Unknown".to_string())
                }
            };

            if is_me {
                commands
                    .spawn((
                        RemotePlayer {
                            connection_id: update.connection_id,
                            target: pos,
                            prev: pos,
                        },
                        PlayerStats { hp: 100, mp: 100, level: 0, xp: 0 },
                        SelfPlayer,
                        Mesh2d(meshes.add(Circle::new(16.0))),
                        MeshMaterial2d(materials.add(ColorMaterial::from_color(color))),
                        Transform::from_translation(pos.extend(0.0)),
                    ))
                    .with_children(|parent| {
                        let label_text = format_remote_player_label(&name, 0);
                        // 1. Spawn the Text Label Child
                        parent.spawn((
                            Text2d::new(label_text),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            // Position it slightly higher in Z-space than the circle so text is on top
                            Transform::from_translation(Vec3::new(0.0, 32.0, 2.0)),
                            RemotePlayerLabel {
                                connection_id: update.connection_id,
                                display_name: name,
                            },
                        ));

                        // Spawn HP/Mana stats text below the name
                        parent.spawn((
                            Text2d::new("HP: 50/100 | MP: 100/100"),
                            TextFont {
                                font_size: 10.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.7, 0.7, 0.7)),
                            Transform::from_translation(Vec3::new(0.0, 20.0, 2.0)),
                            RemotePlayerStatsLabel {
                                connection_id: update.connection_id,
                            },
                        ));

                        // 2. Spawn the Transparent Circle Child
                        parent.spawn((
                            Mesh2d(meshes.add(Circle::new(500.0))),
                            MeshMaterial2d(
                                materials.add(ColorMaterial::from_color(Color::srgba(
                                    1.0, 1.0, 1.0, 0.05,
                                ))),
                            ),
                        ));
                    });
            } else {
                commands
                    .spawn((
                        RemotePlayer {
                            connection_id: update.connection_id,
                            target: pos,
                            prev: pos,
                        },
                        PlayerStats { hp: 100, mp: 100, level: 0, xp: 0 },
                        Mesh2d(meshes.add(Circle::new(16.0))),
                        MeshMaterial2d(materials.add(ColorMaterial::from_color(color))),
                        Transform::from_translation(pos.extend(0.0)),
                    ))
                    .with_children(|parent| {
                        let label_text = format_remote_player_label(&name, 0);
                        parent.spawn((
                            Text2d::new(label_text),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            Transform::from_translation(Vec3::new(0.0, 32.0, 1.0)),
                            RemotePlayerLabel {
                                connection_id: update.connection_id,
                                display_name: name,
                            },
                        ));

                        // Spawn HP/Mana stats text below the name
                        parent.spawn((
                            Text2d::new("HP: 50/100 | MP: 100/100"),
                            TextFont {
                                font_size: 10.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.7, 0.7, 0.7)),
                            Transform::from_translation(Vec3::new(0.0, 20.0, 1.0)),
                            RemotePlayerStatsLabel {
                                connection_id: update.connection_id,
                            },
                        ));
                    });
            }
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
        let new_pos = Vec2::new(
            update.payload.position[0] as f32,
            update.payload.position[1] as f32,
        );
        for (mut remote, mut transform) in &mut query {
            if remote.connection_id == update.connection_id {
                let dist = (new_pos - remote.target).length();
                if dist > POSITION_DELTA_THRESHOLD {
                    if dist > 150.0 || new_pos == Vec2::ZERO {
                        // Instant teleport
                        transform.translation = new_pos.extend(transform.translation.z);
                        remote.prev = new_pos;
                        remote.target = new_pos;
                    } else {
                        remote.prev = remote.target;
                        remote.target = new_pos;
                    }
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

/// Update the text shown under each remote player to include its resolved name.
fn update_remote_player_labels(
    query: Query<(&RemotePlayer, &PlayerStats, &Children)>,
    mut labels: Query<(&mut RemotePlayerLabel, &mut Text2d)>,
    discovered: Res<DiscoveredEntities>,
) {
    for (remote, stats, children) in &query {
        for child in children.iter() {
            if let Ok((mut tag, mut text)) = labels.get_mut(child) {
                if tag.connection_id == remote.connection_id {
                    if let Some(resolved_name) = discovered.0.get(&remote.connection_id) {
                        if tag.display_name != *resolved_name {
                            tag.display_name = resolved_name.clone();
                        }
                    }
                    text.0 = format_remote_player_label(&tag.display_name, stats.level);
                }
            }
        }
    }
}

fn format_remote_player_label(name: &str, level: i32) -> String {
    format!("[Lvl {}] {}", level, name)
}

/// Spawn a top-left debug overlay showing session info. Cleared with the rest
/// of the game scene when leaving InGame (all entities have GameSceneRoot).
fn spawn_debug_hud(
    mut commands: Commands,
    session: Res<GameSession>,
    broker_conn: Option<Res<BrokerConn>>,
    mut events: MessageReader<AuthorityDebugPacketReceived>,
    query: Query<Entity, With<DebugUI>>,
    player_query: Query<(&Transform, &PlayerStats), With<SelfPlayer>>,
) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }

    let connection_id = broker_conn
        .map(|r| r.0.connection_id.to_string())
        .unwrap_or_else(|| "—".to_string());

    let (pos_str, lvl, xp) = if let Some((transform, stats)) = player_query.iter().next() {
        let p = transform.translation.truncate();
        (format!("({:0.1}, {:0.1})", p.x, p.y), stats.level, stats.xp)
    } else {
        ("—".to_string(), 0, 0)
    };

    // 1. Initialize as an owned, mutable String
    let mut debug_info = String::new();

    // Append all ids in authority debug packets to debug_info
    for event in events.read() {
        let sender_id = event.payload.sender_id;

        // 2. Use the write! macro to efficiently append to the String
        use std::fmt::Write;
        let _ = write!(
            debug_info,
            "Authority Debug Packets from connections {}\n",
            sender_id
        );
    }

    let info = format!(
        "Player    : [Lvl {}] {}\nXP        : {}\nConnection ID : {}\nCoordinates : {}\n{}",
        lvl, session.username, xp, connection_id, pos_str, debug_info,
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
            DebugUI,
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

fn handle_disconnect(
    mut commands: Commands,
    mut events: MessageReader<DisconnectReceived>,
    mut query: Query<(Entity, &RemotePlayer), Without<FollowCamera>>,
) {
    for event in events.read() {
        let connection_id = event.entity_id;
        for (entity, remote) in &mut query {
            if remote.connection_id == connection_id {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn delete_entities_outside_of_interest(
    mut commands: Commands,
    query_player: Query<(Entity, &RemotePlayer, &Transform, &SelfPlayer)>,
    query_non_player: Query<(Entity, &RemotePlayer, &Transform), Without<SelfPlayer>>,
) {
    let player_position = if let Some((_, _remote, transform, _)) = query_player.iter().next() {
        transform.translation.truncate()
    } else {
        //print!("No player entity found for delete_entities_outside_of_interest system.\n");
        return;
    };

    //print!("Player position: {:?}\n", player_position);

    for (entity, _remote, transform) in &query_non_player {
        let pos = transform.translation.truncate();
        if (pos - player_position).length() > 500.0 {
            commands.entity(entity).despawn();
        }
    }
}

fn spawn_remote_fireballs(
    mut commands: Commands,
    mut events: MessageReader<super::net::AbilityCastReceived>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    player_query: Query<(&RemotePlayer, &Transform)>,
    self_query: Query<&Transform, With<SelfPlayer>>,
    broker_conn: Option<Res<BrokerConn>>,
) {
    let my_id = broker_conn.map(|b| b.0.connection_id);

    for ev in events.read() {
        if ev.ability_type == common::ability_type::AbilityType::Fireball {
            let raw_direction = ev.direction.unwrap_or(Vec2::X);
            let direction = raw_direction.normalize();

            let mut spawn_pos = player_query
                .iter()
                .find(|(p, _)| p.connection_id == ev.caster_id)
                .map(|(_, t)| t.translation.truncate());

            // If it's your own fireball and the query missed you, fall back to the explicit SelfPlayer transform
            if spawn_pos.is_none() && my_id == Some(ev.caster_id) {
                if let Ok(self_transform) = self_query.single() {
                    spawn_pos = Some(self_transform.translation.truncate());
                }
            }

            let final_base_pos = spawn_pos.unwrap_or(Vec2::ZERO);

            // Offset the fireball slightly in the direction of travel so it doesn't overlap
            let offset = direction * 26.0;
            let final_spawn_pos = final_base_pos + offset;

            commands.spawn((
                ClientProjectile {
                    direction,
                    speed: 400.0,
                },
                Mesh2d(meshes.add(Circle::new(8.0))),
                MeshMaterial2d(
                    materials.add(ColorMaterial::from_color(Color::srgb(1.0, 0.3, 0.0))),
                ), // Bright Orange tint
                Transform::from_translation(final_spawn_pos.extend(7.0)),
                GameSceneRoot,
            ));

            tracing::info!("Fireball spawned at {:?}", final_spawn_pos);
        }
    }
}

fn update_projectiles(mut query: Query<(&ClientProjectile, &mut Transform)>, time: Res<Time>) {
    let dt = time.delta_secs();
    for (proj, mut transform) in &mut query {
        let movement = proj.direction * proj.speed * dt;
        transform.translation.x += movement.x;
        transform.translation.y += movement.y;
    }
}

fn handle_attribute_updates(
    mut events: MessageReader<AttributeUpdatedReceived>,
    mut query: Query<(&mut PlayerStats, &RemotePlayer)>,
) {
    for ev in events.read() {
        for (mut stats, player) in &mut query {
            if player.connection_id == ev.entity_id {
                match ev.attribute {
                    common::attribute_type::AttributeType::HealthPoints => {
                        stats.hp = ev.new_value;
                    }
                    common::attribute_type::AttributeType::ManaPoints => {
                        stats.mp = ev.new_value;
                    }
                }
            }
        }
    }
}

fn handle_xp_and_level_updates(
    mut events_xp: MessageReader<XPEarnedReceived>,
    mut events_lvl: MessageReader<LevelUpReceived>,
    mut query: Query<(&mut PlayerStats, &RemotePlayer)>,
) {
    for ev in events_xp.read() {
        for (mut stats, player) in &mut query {
            if player.connection_id == ev.entity_id {
                stats.xp = ev.xp_gained as i32;
            }
        }
    }
    for ev in events_lvl.read() {
        for (mut stats, player) in &mut query {
            if player.connection_id == ev.entity_id {
                stats.level = ev.new_level as i32;
            }
        }
    }
}

fn update_player_stats_labels(
    players: Query<(&RemotePlayer, &PlayerStats, &Children)>,
    mut labels: Query<(&RemotePlayerStatsLabel, &mut Text2d)>,
) {
    for (remote, stats, children) in &players {
        for child in children.iter() {
            if let Ok((tag, mut text)) = labels.get_mut(child) {
                if tag.connection_id == remote.connection_id {
                    let new_text = format!("HP: {}/100 | MP: {}/100", stats.hp, stats.mp);
                    if text.0 != new_text {
                        text.0 = new_text;
                    }
                }
            }
        }
    }
}
