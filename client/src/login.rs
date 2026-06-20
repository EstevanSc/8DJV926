use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use futures_lite::future;
use serde::{Deserialize, Serialize};

use super::{GameSession, GameState};

pub struct LoginPlugin;

impl Plugin for LoginPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FormState>()
            .init_resource::<PendingSubmit>()
            .add_systems(OnEnter(GameState::Login), setup_login_ui)
            .add_systems(OnExit(GameState::Login), (teardown_login_ui, reset_form))
            .add_systems(
                Update,
                (
                    handle_field_click,
                    update_focus_visuals,
                    handle_keyboard_input,
                    handle_submit,
                    poll_join_task,
                    tick_success_timer,
                )
                    .run_if(in_state(GameState::Login)),
            );
    }
}

// ---------------------------------------------------------------------------
// Form state
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
struct FormState {
    username: String,
    password: String,
    focused: FocusedField,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum FocusedField {
    #[default]
    Username,
    Password,
}

/// Set to true by the Enter key; consumed (reset) by handle_submit.
#[derive(Resource, Default)]
struct PendingSubmit(bool);

// ---------------------------------------------------------------------------
// UI markers
// ---------------------------------------------------------------------------

#[derive(Component)]
struct LoginRoot;

#[derive(Component)]
struct UsernameFieldButton;

#[derive(Component)]
struct PasswordFieldButton;

#[derive(Component)]
struct UsernameDisplay;

#[derive(Component)]
struct PasswordDisplay;

#[derive(Component)]
struct JoinButton;

#[derive(Component)]
struct StatusText;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Inserted on successful login; counts down before transitioning to Connecting.
#[derive(Resource)]
struct TransitionTimer(Timer);

// ---------------------------------------------------------------------------
// Async join task
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginResponse {
    broker_ip: String,
    broker_port: u16,
    player_spawn_position: [f32; 2],
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Component)]
struct JoinTask(Task<Result<(String, LoginResponse), String>>);

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

fn setup_login_ui(mut commands: Commands) {
    commands.spawn((Camera2d, LoginRoot));

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(16.0),
                ..default()
            },
            LoginRoot,
        ))
        .with_children(|p| {
            // Title
            p.spawn((
                Text::new("Extraction MMO"),
                TextFont {
                    font_size: 40.0,
                    ..default()
                },
            ));

            // Username row
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new("Username:"),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                ));
                row.spawn((
                    Button,
                    Node {
                        width: Val::Px(240.0),
                        height: Val::Px(36.0),
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.2, 0.2, 0.45)), // focused by default
                    UsernameFieldButton,
                ))
                .with_children(|f| {
                    f.spawn((
                        Text::new("|"),
                        TextFont {
                            font_size: 20.0,
                            ..default()
                        },
                        UsernameDisplay,
                    ));
                });
            });

            // Password row
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new("Password:"),
                    TextFont {
                        font_size: 22.0,
                        ..default()
                    },
                ));
                row.spawn((
                    Button,
                    Node {
                        width: Val::Px(240.0),
                        height: Val::Px(36.0),
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.15, 0.15, 0.15)),
                    PasswordFieldButton,
                ))
                .with_children(|f| {
                    f.spawn((
                        Text::new("|"),
                        TextFont {
                            font_size: 20.0,
                            ..default()
                        },
                        PasswordDisplay,
                    ));
                });
            });

            // Join button
            p.spawn((
                Button,
                Node {
                    padding: UiRect::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.2, 0.6, 0.2)),
                JoinButton,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Login"),
                    TextFont {
                        font_size: 24.0,
                        ..default()
                    },
                ));
            });

            // Status / hint line
            p.spawn((
                Text::new("Click a field or Tab to switch focus  •  Enter or Join to login"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
                StatusText,
            ));
        });
}

fn teardown_login_ui(mut commands: Commands, roots: Query<Entity, With<LoginRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn_related::<Children>();
        commands.entity(entity).despawn();
    }
}

fn reset_form(mut form: ResMut<FormState>, mut pending: ResMut<PendingSubmit>) {
    *form = FormState::default();
    pending.0 = false;
}

/// Update focused field when a field button is clicked.
fn handle_field_click(
    mut form: ResMut<FormState>,
    username_q: Query<&Interaction, (Changed<Interaction>, With<UsernameFieldButton>)>,
    password_q: Query<&Interaction, (Changed<Interaction>, With<PasswordFieldButton>)>,
) {
    for &interaction in &username_q {
        if interaction == Interaction::Pressed {
            form.focused = FocusedField::Username;
        }
    }
    for &interaction in &password_q {
        if interaction == Interaction::Pressed {
            form.focused = FocusedField::Password;
        }
    }
}

/// Highlight the currently focused field (runs every frame — cheap).
fn update_focus_visuals(
    form: Res<FormState>,
    mut username_bg: Query<
        &mut BackgroundColor,
        (With<UsernameFieldButton>, Without<PasswordFieldButton>),
    >,
    mut password_bg: Query<
        &mut BackgroundColor,
        (With<PasswordFieldButton>, Without<UsernameFieldButton>),
    >,
) {
    if let Ok(mut bg) = username_bg.single_mut() {
        bg.0 = if form.focused == FocusedField::Username {
            Color::srgb(0.2, 0.2, 0.45)
        } else {
            Color::srgb(0.15, 0.15, 0.15)
        };
    }
    if let Ok(mut bg) = password_bg.single_mut() {
        bg.0 = if form.focused == FocusedField::Password {
            Color::srgb(0.2, 0.2, 0.45)
        } else {
            Color::srgb(0.15, 0.15, 0.15)
        };
    }
}

/// Route keyboard events to the focused field and update the display text.
fn handle_keyboard_input(
    mut form: ResMut<FormState>,
    mut pending_submit: ResMut<PendingSubmit>,
    mut key_events: MessageReader<KeyboardInput>,
    mut username_display: Query<
        &mut Text,
        (
            With<UsernameDisplay>,
            Without<PasswordDisplay>,
            Without<StatusText>,
        ),
    >,
    mut password_display: Query<
        &mut Text,
        (
            With<PasswordDisplay>,
            Without<UsernameDisplay>,
            Without<StatusText>,
        ),
    >,
) {
    let mut dirty = false;

    for event in key_events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }

        match &event.logical_key {
            Key::Character(ch) => {
                let s: &str = ch.as_str();
                if s.chars().all(|c| !c.is_control()) {
                    match form.focused {
                        FocusedField::Username => form.username.push_str(s),
                        FocusedField::Password => form.password.push_str(s),
                    }
                    dirty = true;
                }
            }
            Key::Backspace => {
                match form.focused {
                    FocusedField::Username => {
                        form.username.pop();
                    }
                    FocusedField::Password => {
                        form.password.pop();
                    }
                }
                dirty = true;
            }
            Key::Tab => {
                form.focused = match form.focused {
                    FocusedField::Username => FocusedField::Password,
                    FocusedField::Password => FocusedField::Username,
                };
            }
            Key::Enter => {
                pending_submit.0 = true;
            }
            _ => {}
        }
    }

    if dirty {
        if let Ok(mut text) = username_display.single_mut() {
            text.0 = if form.username.is_empty() {
                "|".to_string()
            } else {
                format!("{}|", form.username)
            };
        }
        if let Ok(mut text) = password_display.single_mut() {
            text.0 = if form.password.is_empty() {
                "|".to_string()
            } else {
                format!("{}|", "*".repeat(form.password.len()))
            };
        }
    }
}

/// Handle Join button click or Enter key — validate then fire the HTTP request.
fn handle_submit(
    mut commands: Commands,
    form: Res<FormState>,
    mut pending_submit: ResMut<PendingSubmit>,
    join_btn_q: Query<&Interaction, (Changed<Interaction>, With<JoinButton>)>,
    mut status_query: Query<(&mut Text, &mut TextColor), With<StatusText>>,
    gatekeeper_url: Option<Res<GatekeeperUrl>>,
    existing_tasks: Query<(), With<JoinTask>>,
) {
    let button_pressed = join_btn_q.iter().any(|&i| i == Interaction::Pressed);
    let should_submit = button_pressed || pending_submit.0;
    pending_submit.0 = false;

    if !should_submit {
        return;
    }

    // Don't queue a second request while one is already in flight.
    if !existing_tasks.is_empty() {
        return;
    }

    let username = form.username.trim().to_string();
    let password = form.password.clone();

    if username.is_empty() {
        if let Ok((mut text, mut color)) = status_query.single_mut() {
            text.0 = "Username cannot be empty.".to_string();
            color.0 = Color::srgb(1.0, 0.4, 0.4);
        }
        return;
    }

    if password.is_empty() {
        if let Ok((mut text, mut color)) = status_query.single_mut() {
            text.0 = "Password cannot be empty.".to_string();
            color.0 = Color::srgb(1.0, 0.4, 0.4);
        }
        return;
    }

    if let Ok((mut text, mut color)) = status_query.single_mut() {
        text.0 = "Contacting gatekeeper...".to_string();
        color.0 = Color::srgb(0.8, 0.8, 0.2);
    }

    let url = gatekeeper_url
        .as_ref()
        .map(|r| r.0.clone())
        .unwrap_or_else(|| "http://localhost:3000".to_string());

    let task_pool = AsyncComputeTaskPool::get();
    let task = task_pool.spawn(async move {
        // reqwest requires a Tokio runtime; AsyncComputeTaskPool uses async-executor
        // (no Tokio context), so we build a dedicated single-threaded runtime here.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        let username_clone = username.clone();
        rt.block_on(async {
            let client = reqwest::Client::new();
            let resp = client
                .post(format!("{url}/login"))
                .json(&LoginRequest { username, password })
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if resp.status().is_success() {
                resp.json::<LoginResponse>()
                    .await
                    .map(|r| (username_clone, r))
                    .map_err(|e| e.to_string())
            } else {
                let status = resp.status();
                let msg = resp
                    .json::<ErrorResponse>()
                    .await
                    .map(|e| e.error)
                    .unwrap_or_else(|_| format!("Server error {}", status));
                Err(msg)
            }
        })
    });

    commands.spawn(JoinTask(task));
}

fn poll_join_task(
    mut commands: Commands,
    mut tasks: Query<(Entity, &mut JoinTask)>,
    mut status_query: Query<(&mut Text, &mut TextColor), With<StatusText>>,
) {
    for (entity, mut task) in &mut tasks {
        if let Some(result) = future::block_on(future::poll_once(&mut task.0)) {
            commands.entity(entity).despawn();
            if let Ok((mut text, mut color)) = status_query.single_mut() {
                match result {
                    Ok((username, resp)) => {
                        commands.insert_resource(GameSession {
                            username,
                            broker_ip: resp.broker_ip.clone(),
                            broker_port: resp.broker_port,
                            player_spawn_position: resp.player_spawn_position,
                            /*  legacy
                            server_ip: resp.server.ip.clone(),
                            server_port: resp.server.port,
                            server_zone: resp.server.zone.clone(),
                            */
                        });
                        text.0 = format!(
                            "Login successful!\nPlayer Spawn Position: ({}, {})\nBroker: {}:{}\nConnecting in 2s...",
                            resp.player_spawn_position[0], resp.player_spawn_position[1], resp.broker_ip, resp.broker_port,
                        );
                        color.0 = Color::srgb(0.2, 0.9, 0.2);
                        commands.insert_resource(TransitionTimer(Timer::from_seconds(
                            2.0,
                            TimerMode::Once,
                        )));
                    }
                    Err(e) => {
                        text.0 = format!("Join failed: {e}");
                        color.0 = Color::srgb(1.0, 0.4, 0.4);
                    }
                }
            }
        }
    }
}

fn tick_success_timer(
    mut commands: Commands,
    time: Res<Time>,
    timer: Option<ResMut<TransitionTimer>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Some(mut t) = timer else { return };
    if t.0.tick(time.delta()).just_finished() {
        commands.remove_resource::<TransitionTimer>();
        next_state.set(GameState::Connecting);
    }
}

/// Optional resource to override the gatekeeper base URL (e.g. from env).
#[derive(Resource)]
pub struct GatekeeperUrl(pub String);
