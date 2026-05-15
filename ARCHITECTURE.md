# Architecture & Test Guide

## Overview

The project is a Rust workspace with three members:

| Crate | Kind | Description |
|---|---|---|
| `common` | library | Shared types: packets, Redis key helpers, constants |
| `crates` | binary | Bevy game client |
| `crates/gatekeeper` | binary | Axum HTTP authentication server |

Redis is the only shared data store. The dedicated game server (QUIC) is not yet implemented — that connection is stubbed on the client side.

---

## Component: Client (`crates/`)

A Bevy 0.18 desktop application. Runs locally; never inside Docker.

### State machine

```
Login  ──(success)──▶  Connecting  ──(done)──▶  InGame
```

### Plugins

#### `LoginPlugin` (`client/login.rs`)
Owns the `Login` state entirely.

- **`FormState`** (resource) — holds `username`, `password`, and which field has focus (`FocusedField::Username | Password`).
- **`PendingSubmit`** (resource) — set to `true` by the Enter key; consumed by `handle_submit`.
- **UI systems:**
  - `setup_login_ui` — spawns two clickable field boxes, a Login button, and a status text node.
  - `handle_field_click` — clicking a box sets focus on it.
  - `update_focus_visuals` — highlights the focused field blue; the other goes dark.
  - `handle_keyboard_input` — reads `KeyboardInput` messages. Handles printable characters, Backspace, Tab (switches focus), and Enter (sets `PendingSubmit`). Password is displayed as `****`.
  - `handle_submit` — validates non-empty fields, spawns an async `reqwest` task that POSTs `{"username", "password"}` to `http://localhost:3000/login`. Task type: `Task<Result<(String, LoginResponse), String>>` (username is carried alongside the response).
  - `poll_join_task` — polls the async task each frame. On success: displays green response text (player_id + server info), inserts `GameSession`, starts a `TransitionTimer` (2 s).
  - `tick_success_timer` — after 2 s transitions to `GameState::Connecting`.

- **`GameSession`** (resource, inserted on successful login):
  ```
  player_id, username, server_ip, server_port, server_zone
  ```

#### `ClientNetPlugin` (`client/net.rs`)
Owns the `Connecting` and `InGame` states for networking.

- **`QuicRuntime`** — a persistent `tokio::Runtime` kept alive for future QUIC work.
- **`start_connect`** (on enter `Connecting`) — currently **stubbed**: derives a deterministic `entity_id` from the player_id bytes instead of opening a real QUIC connection. Will be replaced with a dial to `session.server_ip:session.server_port` once the game server is implemented.
- **`poll_connect_task`** — polls the stub task; on completion inserts `MyEntityId` and transitions to `InGame`.
- **`receive_packets`** — listens for `DatagramReceiver` messages in `InGame` (also stubbed pending real QUIC).

#### `InterpolationPlugin` (`client/interpolation.rs`)
Owns rendering inside `InGame`.

- **`spawn_floor`** — camera + a static floor mesh.
- **`spawn_debug_hud`** — semi-transparent overlay (top-left) showing:
  ```
  Player    : <username>
  Player ID : <uuid>
  Entity ID : <number>
  Server    : <ip>:<port>
  Zone      : <zone>
  ```
- **`spawn_remote_players`** — spawns a circle for each entity_id seen in a `PositionBatch`. Own player = green, others = blue.
- **`interpolate_remote_players`** — smoothly moves circles toward their target positions each frame.

#### `ClientInputPlugin` (`client/input.rs`)
Reads keyboard/gamepad input and sends movement packets (active in `InGame`).

---

## Component: Gatekeeper (`crates/gatekeeper/`)

An Axum 0.8 HTTP server. Runs in Docker on port `3000`. Talks to Redis over the internal Docker network.

### Startup (`main.rs`)

1. Installs the `ring` rustls crypto provider.
2. Initialises `tracing` (log level from `RUST_LOG` env var).
3. Reads `REDIS_URL` from the environment (default: `redis://127.0.0.1:6379`).
4. Creates a `deadpool-redis` connection pool.
5. Binds Axum on `0.0.0.0:3000`.

### Routes

| Method | Path | Handler |
|---|---|---|
| `GET` | `/health` | Returns `"ok"` |
| `POST` | `/login` | `routes::join::handler` |

### `POST /login` flow (`routes/join.rs`)

```
Client JSON body
  { "username": "alice", "password": "1234" }
        │
        ▼
  Validate username non-empty  →  400 Bad Request if empty
        │
        ▼
  redis_ops::find_available_server()
    SMEMBERS servers:active
    for each id:
      HGETALL server:<id>
      return first where status == "available"
        │
        ▼  None → 503 Service Unavailable
        ▼  Some(server)
  redis_ops::increment_player_count()
    HINCRBY server:<id> players 1
        │
        ▼
  Response 200 OK
  {
    "player_id": "<uuid-v4>",
    "server": { "ip": "...", "port": ..., "zone": "..." }
  }
```

Authentication currently accepts any non-empty username. Real auth is a future milestone.

### Redis schema (`common/src/redis_keys.rs`)

| Key | Type | Fields |
|---|---|---|
| `servers:active` | SET | Server IDs (strings) |
| `server:<id>` | HASH | `ip`, `port`, `zone`, `status`, `players` |

`status` must be `"available"` for the gatekeeper to assign a player to it.

---

## Login flow end-to-end

```
┌─────────────────────────────────────────────────────────────────┐
│  Client (Bevy)                                                  │
│                                                                 │
│  [Login screen]                                                 │
│   User types username + password, presses Enter                 │
│   handle_submit spawns async reqwest task                       │
│         │                                                       │
│         │  POST http://localhost:3000/login                     │
│         │  { "username": "alice", "password": "1234" }         │
└─────────┼───────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│  Gatekeeper (Axum, Docker :3000)                                │
│                                                                 │
│  routes::join::handler                                          │
│   → SMEMBERS servers:active          (Redis)                    │
│   → HGETALL  server:<id>             (Redis, per candidate)     │
│   → HINCRBY  server:<id> players 1   (Redis, chosen server)     │
│   → 200 { player_id, server: {ip, port, zone} }                │
└─────────┼───────────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│  Client (Bevy) — continued                                      │
│                                                                 │
│  poll_join_task receives response                               │
│   → inserts GameSession { player_id, username,                  │
│                           server_ip, server_port, server_zone } │
│   → shows green confirmation text (2 s)                         │
│   → transitions to GameState::Connecting                        │
│                                                                 │
│  start_connect  [STUB — no real QUIC yet]                       │
│   → derives entity_id from player_id bytes                      │
│   → inserts MyEntityId                                          │
│   → transitions to GameState::InGame                            │
│                                                                 │
│  [InGame]                                                       │
│   → debug HUD shows session info                                │
│   → player circle rendered green                                │
└─────────────────────────────────────────────────────────────────┘
```

---

## Test procedure

### Prerequisites

- Docker Desktop running
- Rust toolchain installed (`cargo`)

### 1. Start services

```powershell
docker compose up -d
```

### 2. Verify containers are healthy

```powershell
docker compose ps
```

Both `redis-1` and `gatekeeper-1` should show status `running`.

### 3. Seed Redis with a test server

```powershell
docker compose exec redis redis-cli SADD servers:active test-1
docker compose exec redis redis-cli HSET server:test-1 ip 127.0.0.1 port 7001 zone zone_A status available players 0
```

### 4. Test the health endpoint

```powershell
Invoke-RestMethod -Uri http://localhost:3000/health
```

Expected response: `ok`

### 5. Test the login endpoint

```powershell
Invoke-RestMethod -Method Post -Uri http://localhost:3000/login `
  -ContentType "application/json" `
  -Body '{"username":"alice","password":"1234"}' | Format-List
```

Expected:

```
player_id : <some-uuid>
server    : @{ip=127.0.0.1; port=7001; zone=zone_A}
```

### 6. Verify the player count incremented

```powershell
docker compose exec redis redis-cli HGETALL server:test-1
```

`players` should now be `1`. Call `/login` again with a different username — it becomes `2`.

### 7. Check gatekeeper logs

```powershell
docker compose logs gatekeeper
```

Expected: one `INFO` line per successful login:

```
INFO gatekeeper::routes::join: Player 'alice' assigned → 127.0.0.1:7001 (zone_A) player_id=<uuid> server_id=test-1
```

### 8. Run the client

```powershell
cargo run -p client
```

- Login screen: two text fields (click or Tab to switch focus), Enter or Login button to submit. Password shows as `****`.
- On success: green text shows `player_id` + server info for 2 seconds, then transitions.
- In-game: debug HUD (top-left) shows Player, Player ID, Entity ID, Server, and Zone. Own player circle is green.

### 9. Teardown

```powershell
docker compose down
```

---

## Known limitations / future work

| Item | Status |
|---|---|
| Real QUIC connection to game server | `TODO` in `client/net.rs::start_connect` |
| Real password authentication | Gatekeeper accepts any non-empty username |
| Server capacity limit (max players) | Not enforced — status stays `"available"` indefinitely |
