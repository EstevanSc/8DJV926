# Architecture & Test Guide

## Overview

The project is a Rust workspace with six members:

| Crate | Kind | Description |
|---|---|---|
| `common` | library | Shared types: packets, Redis key helpers, constants |
| `client` | binary | Bevy game client |
| `gatekeeper` | binary | Axum HTTP authentication server |
| `GameServer` | binary | Bevy headless dedicated game server (QUIC) |
| `Orchestrator` | binary | Axum server lifecycle manager and heartbeat listener |
| `game_sockets` | library | QUIC/TCP/UDP networking abstraction used by client and GameServer |

Redis is the shared data store between gatekeeper and orchestrator. The game server registers itself with the orchestrator via UDP heartbeats; the gatekeeper reads that state to route players.

---

## Component: Client (`client/`)

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

## Component: Gatekeeper (`gatekeeper/`)

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

## Component: GameServer (`GameServer/`)

A Bevy headless binary (`MinimalPlugins`). Runs inside Docker; never renders.

### Startup

1. Reads `DS_PORT` (default `7777`), `DS_ZONE`, `MAX_PLAYERS`, and `ORCH_HOST` from env.
2. Binds a QUIC socket via `game_sockets::GamePeer` / `QuicBackend`.
3. Schedules `receive_packets` and `send_heartbeat` every frame via Bevy `Update`.

### Heartbeat (`src/heartbeat.rs`)

Every 5 s, serialises a `Heartbeat` struct to JSON and sends it via UDP to the orchestrator address. Includes `id`, `ip`, `port`, `zone`, `player_count`, `max_players`.

### Message protocol (`src/messages.rs`)

```
GameMessage::Join    { username }
GameMessage::Welcome { player_id }
```

Encoded with `wincode` (binary schema). On `Join`, the server assigns the connection UUID as the player ID and replies with `Welcome`.

### Test client (`src/bin/test_client.rs`)

Stand-alone binary that opens a QUIC connection to `127.0.0.1:7777`, sends `Join { username: "Alice_Tester" }`, and prints the `Welcome` response.

---

## Component: Orchestrator (`Orchestrator/`)

An Axum 0.8 HTTP server. Runs in Docker on port `8081`. Listens for UDP heartbeats on port `7000`. Talks to Redis.

### Startup (`src/main.rs`)

1. Loads config from env / `.env` file via `dotenv`.
2. Connects to Redis and verifies with `PING`.
3. Spawns two background tasks: heartbeat listener and scaler.
4. Binds Axum on `0.0.0.0:<PORT>`.

### Routes

| Method | Path | Handler |
|---|---|---|
| `GET` | `/api/health` | Returns service status |

### Heartbeat listener (`src/services/heartbeat_listener.rs`)

Binds a UDP socket on `ORCH_PORT` (default `7000`). Each JSON packet from a GameServer is parsed into a `Heartbeat` and written to Redis as a hash with a TTL.

### Scaler (`src/services/scaler.rs`)

Polls Redis every `SCALER_INTERVAL_SECONDS` seconds. Ensures at least `HOT_SERVERS_MIN` servers are running by spawning new `DS_BINARY_PATH` processes if needed.

---

## Component: game_sockets (`game_sockets/`)

A library crate providing a transport-agnostic networking API used by both the client and GameServer.

### Key types

| Type | Description |
|---|---|
| `GamePeer` | Entry point — wraps a `GameSocketBackend` and exposes `listen`, `connect`, `send`, `poll` |
| `GameConnection` | Handle for a single peer connection (UUID) |
| `GameStream` | Logical stream within a connection; stream ID encodes reliability in the low 2 bits |
| `GameNetworkEvent` | Events emitted by `poll`: `Connected`, `Disconnected`, `Message`, `StreamCreated` |
| `BackendCommand` | Commands sent to the backend thread: `Bind`, `Connect`, `Send`, `CreateStream`, … |

### Backends

| Module | Protocol |
|---|---|
| `QuicBackend` | QUIC over UDP via `quinn 0.10` + `rustls 0.21` (self-signed TLS) |
| `TcpBackend` | Length-delimited TCP frames via `tokio-util::codec::LengthDelimitedCodec` |
| `UdpBackend` | Raw UDP datagrams |

> **Note:** `game_sockets` intentionally pins `quinn 0.10` / `rustls 0.21` / `rcgen 0.11` because `quic_protocol.rs` uses the pre-0.22 rustls API (`Certificate`, `PrivateKey`, `ServerCertVerifier`). The rest of the workspace uses `quinn 0.11` / `rustls 0.23`; Cargo compiles both versions without conflict.

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
| Real QUIC connection from client to GameServer | `TODO` in `client/net.rs::start_connect` — currently stubbed |
| Real password authentication | Gatekeeper accepts any non-empty username |
| Server capacity limit (max players) | Not enforced by gatekeeper — status stays `"available"` indefinitely |
| Heartbeat struct deduplication | `GameServer/src/heartbeat.rs` is a temporary copy; should use `common` once merged |
| `game_sockets` quinn/rustls upgrade | Pinned to quinn 0.10 / rustls 0.21; upgrade requires rewriting `quic_protocol.rs` |
