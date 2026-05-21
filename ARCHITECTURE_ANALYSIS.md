# Repository Architecture Analysis

This document analyzes each folder in the repository and explains how the components work internally and how they interact as a distributed game platform.

## 1) Top-Level Repository Overview

The workspace is a Rust monorepo with multiple crates:
- `client`: Bevy game client with login UI, networking, input, and interpolation.
- `common`: Shared types and helpers (packets, heartbeat schema, Redis client, constants).
- `game_sockets`: Transport abstraction library (QUIC, TCP, UDP backends).
- `GameServer`: Dedicated game server running simulation + networking + heartbeat emission.
- `gatekeeper`: Login/auth and server-assignment HTTP service.
- `Orchestrator`: Fleet manager, heartbeat listener, and Docker scaler.

Infrastructure around those crates:
- root `docker-compose.yml`: runs Redis + Gatekeeper + Orchestrator, and defines game-server image.
- `docs`: runtime screenshots.
- `target`: Rust build artifacts.
- `.cargo`: cargo build profile tuning.

---

## 2) Folder-by-Folder Analysis

## `.cargo`

### Purpose
Holds local Cargo build profile overrides.

### How it works
- `config.toml` sets aggressive release optimization and improves development speed for heavy dependencies:
  - `release`: `opt-level=3`, `lto=thin`, `strip=symbols`.
  - `dev`: `opt-level=1` for local crate, but `opt-level=3` for dependencies (`[profile.dev.package."*"]`), which is helpful for Bevy/physics performance during development.

---

## `client`

### Purpose
Interactive game client (Bevy app) with three phases:
1. login to gatekeeper,
2. connect to assigned game server via QUIC,
3. play and render replicated entities.

### Structure
- `main.rs`: starts `src::run()`.
- `src/mod.rs`: app bootstrap and state machine.
- `src/login.rs`: full login UI and async HTTP auth flow.
- `src/net.rs`: QUIC connection lifecycle and packet handling.
- `src/input.rs`: sends movement input to server.
- `src/interpolation.rs`: spawns/interpolates remote entities from snapshots.

### Internal behavior
1. `run()` initializes Rustls crypto provider and Bevy `DefaultPlugins`.
2. State machine (`GameState`): `Login -> Connecting -> InGame`.
3. Login flow:
   - captures username/password in UI.
   - POST `/login` to gatekeeper.
   - receives `player_id` + target server (`ip`, `port`, `zone`).
   - stores `GameSession` resource and transitions to `Connecting`.
4. Connecting flow:
   - creates `GamePeer` with QUIC backend.
   - connects to server.
   - on `Connected`, sends `GameMessage::Join { username }`.
   - waits for `Welcome { player_id }`.
   - derives local `entity_id` from UUID bytes and moves to `InGame`.
5. In-game flow:
   - `input.rs`: reads keyboard, serializes `PlayerInput`, sends on stream 0.
   - `net.rs`: receives `PositionBatch` snapshots.
   - `interpolation.rs`: creates circles/name tags for unseen entity ids, then lerps transform to latest target position.

### Key design choices
- Uses shared packet types from `common` (`PlayerInput`, `PositionBatch`), reducing wire-protocol drift.
- Uses optimistic interpolation with `POSITION_DELTA_THRESHOLD` to avoid micro-jitter updates.
- Keeps network peer in `Mutex<GamePeer>` for Bevy resource synchronization.

---

## `common`

### Purpose
Shared contract crate used by gatekeeper, orchestrator, server, and client.

### Contents and role
- `packets.rs`:
  - `PositionSnapshot`, `PositionBatch` for state replication.
  - `PlayerInput` for client movement commands.
  - also defines `ConnectRequest` / `AuthAck` structs.
- `heartbeat.rs`:
  - `Heartbeat` schema sent from game server to orchestrator.
- `server_info.rs`:
  - canonical server metadata (`id`, `ip`, `port`, `zone`, `status`, `player_count`, `max_players`).
- `redis_client.rs`:
  - async wrapper over Redis `ConnectionManager` with `scan`, `hset_multiple`, `hget`, `expire`, `hincr`, etc.
- `constants.rs`:
  - transport/visibility tuning values:
    - `POSITION_DELTA_THRESHOLD`.
    - `INTEREST_RADIUS_TILES`.
- `redis_keys.rs`:
  - key helpers like `server:<id>`.

### Why it matters
This crate is the integration backbone. It aligns data models across services and avoids duplicate schema definitions.

---

## `docs`

### Purpose
Documentation assets (screenshots of runtime behavior).

### How it works
Contains PNG captures of:
- orchestrator startup,
- running containers,
- login UI and success,
- gatekeeper logs,
- client welcome,
- heartbeat logs,
- dynamic server orchestration logs.

It supports manual verification and presentation, not runtime logic.

---

## `game_sockets`

### Purpose
Pluggable network transport abstraction exposing a unified API (`GamePeer`) over multiple protocol backends.

### Structure
- `src/lib.rs`: defines core types and command/event channels.
- `src/protocols/quic_protocol.rs`: primary production backend.
- `src/protocols/tcp_protocol.rs`: framed TCP backend.
- `src/protocols/udp_protocol.rs`: custom packetized UDP backend.

### How it works
1. `GamePeer` owns:
   - command sender to backend thread,
   - event receiver from backend thread,
   - stream id allocator.
2. Public API:
   - `listen`, `connect`, `create_stream`, `send`, `poll`, `shutdown`.
3. Backend thread model:
   - each backend implements `GameSocketBackend::run`.
   - backend executes in dedicated thread and can spawn Tokio tasks.

### QUIC backend specifics
- Uses Quinn + Rustls with self-signed cert setup.
- Enables low-latency transport tuning:
  - BBR congestion,
  - low initial RTT,
  - keep-alive,
  - datagram buffer tuning.
- Supports:
  - unreliable datagrams (for high-frequency updates),
  - reliable bidirectional streams (framed messages with length prefix).

### Stream encoding model
`GameStream` packs reliability/order metadata in low bits of stream id (`RELIABILITY_MASK`, `ORDERING_MASK`), giving a compact lane descriptor.

---

## `GameServer`

### Purpose
Dedicated authoritative simulation server for gameplay sessions.

### Structure
- `src/main.rs`: starts Bevy app with `ServerPlugin` + `SimulationPlugin`.
- `src/server.rs`: network bind, packet receive loop, join/welcome handling, heartbeat sending.
- `src/simulation.rs`: physics world, player spawn/despawn/input application, snapshot broadcast.
- `src/char_controller.rs`: movement and grounded-state systems.
- `src/interest.rs`: visibility culling based on radius.
- `src/net.rs`: sim command channel and connection registry helpers.
- `src/messages.rs`: `GameMessage::{Join,Welcome}` contract.
- `src/heartbeat.rs`: local heartbeat struct (currently duplicate of shared schema).
- `src/bin/test_client.rs`: standalone mock QUIC test client.

### Runtime behavior
1. On startup:
   - reads env config (`DS_PORT`, `DS_ID`, `DS_PUBLIC_IP`, `DS_ZONE`, `MAX_PLAYERS`, `ORCH_HOST`).
   - binds QUIC listener.
2. On client connect:
   - stores connection in `ConnectedPlayers`.
3. On `Join` message:
   - derives deterministic `entity_id` from connection UUID.
   - inserts player into registry.
   - pushes `SimCommand::PlayerJoined` into simulation.
   - responds `Welcome { player_id }`.
4. Input handling:
   - receives serialized `PlayerInput` and forwards into simulation via channel.
5. Simulation tick:
   - drains join/leave/input commands.
   - updates physics and player movement.
   - interest-filters visible entities per observer.
   - sends per-client `PositionBatch` on stream 0.
6. Heartbeat:
   - every 5 seconds sends JSON UDP heartbeat to orchestrator (`id`, `ip`, `port`, occupancy, capacity).

### Notable architecture detail
The game server is authoritative for positions and occupancy reality, while gatekeeper uses Redis approximations for routing in-between heartbeat intervals.

---

## `gatekeeper`

### Purpose
Authentication/entry gateway for clients.

### Structure
- `src/main.rs`: starts Axum HTTP on `0.0.0.0:3000`.
- `src/routes/join.rs`: `POST /login` request flow.
- `src/routes/health.rs`: liveness endpoint.
- `src/db.rs`: Supabase REST client wrapper.
- `src/redis_ops.rs`: server discovery and player count increment logic.

### Login workflow
1. Validate non-empty username/password.
2. Query Supabase `PlayerInformation` table:
   - if user exists: password check.
   - else: create user.
3. Query Redis for best server candidate:
   - considers `status in {empty, available}`.
   - ranks `available` before `empty`.
   - among same class, favors highest `player_count` to fill active instances first.
4. Atomically `HINCRBY player_count +1` on selected server.
5. Return JSON:
   - `player_id` (db id as string),
   - selected `ServerInfo` (ip/port/zone/status/capacity).

### Key integration role
Acts as traffic director from identity layer to runtime world instance.

---

## `Orchestrator`

### Purpose
Fleet-control plane for game servers.

### Structure
- `src/main.rs`: service startup and task orchestration.
- `src/config.rs`: environment parsing defaults.
- `src/services/heartbeat_listener.rs`: UDP heartbeat ingestion + Redis persistence.
- `src/services/scaler.rs`: periodic scale up/down loop.
- `src/docker_ops.rs`: Docker API operations for container lifecycle.
- `src/api/health.rs`: health endpoint under `/api/health`.
- `mock_server.sh`: test heartbeat generator.

### Runtime behavior
1. On startup:
   - loads config (`PORT`, `ORCH_PORT`, `REDIS_URL`, `DS_BASE_PORT`, `HOT_SERVERS_MIN`, etc).
   - connects to Redis.
   - starts heartbeat listener task (UDP).
   - connects to Docker daemon and starts scaler task.
   - starts HTTP API.
2. Heartbeat listener:
   - receives JSON UDP payloads from game servers.
   - computes status:
     - `empty` if players = 0,
     - `full` if players >= max,
     - else `available`.
   - writes to `server:<id>` hash in Redis.
   - sets TTL (`HEARTBEAT_TTL_SECONDS`) so stale servers auto-expire.
3. Scaler:
   - at fixed interval (`SCALER_INTERVAL_SECONDS`), scans Redis.
   - if empty servers < `HOT_SERVERS_MIN`: spawn new game-server containers.
   - if empty servers > `HOT_SERVERS_MIN`: stop excess containers and delete Redis keys.
4. Docker spawning:
   - injects env vars (`DS_ID`, `DS_PORT`, `DS_PUBLIC_IP`, `ORCH_HOST`, etc).
   - maps UDP game port to host.
   - pre-registers Redis entry with `status=starting`; later overwritten by heartbeat state.

### Why this is important
Orchestrator closes the loop between observed capacity and desired warm capacity.

---

## `target`

### Purpose
Rust compiler artifacts and incremental cache.

### How it works
Contains build output (`debug`, deps, incremental state). It is generated data and not part of application logic.

---

## Hidden/Meta Folders

## `.git`
Repository history and SCM metadata.

## Root config files
- `Cargo.toml`: workspace members/dependencies and release profile defaults.
- `docker-compose.yml`: multi-service deployment.
- `clippy.toml`: lint policy.
- `rustfmt.toml`: formatting policy.
- `.env`: runtime environment values for compose and services.

---

## 3) Component Interaction Model

## High-Level Interaction Graph

```mermaid
flowchart LR
    A[Client Bevy App] -->|POST /login| B[Gatekeeper HTTP]
    B -->|Read/Create player| C[Supabase REST]
    B -->|Discover server keys| D[Redis]
    B -->|HINCRBY player_count| D
    B -->|LoginResponse player_id + server ip/port| A

    A -->|QUIC join + input + recv snapshots| E[GameServer]
    E -->|UDP heartbeat JSON| F[Orchestrator UDP Listener]
    F -->|HSET server hash + TTL| D

    G[Orchestrator Scaler] -->|SCAN server:*| D
    G -->|Spawn/Stop containers| H[Docker Daemon]
    H -->|Run containers| E

    F -->|/api/health| I[HTTP API]
```

## Detailed Runtime Sequence (Login to Gameplay)

```mermaid
sequenceDiagram
    participant U as User
    participant CL as Client
    participant GK as Gatekeeper
    participant SB as Supabase
    participant RD as Redis
    participant OR as Orchestrator
    participant GS as GameServer

    U->>CL: Enter username/password
    CL->>GK: POST /login
    GK->>SB: find_player(name)
    alt existing user
        SB-->>GK: row
        GK->>GK: verify password
    else new user
        GK->>SB: create_player(name,password)
        SB-->>GK: created row
    end

    GK->>RD: SCAN/HGET server:* status/capacity
    RD-->>GK: candidate servers
    GK->>RD: HINCRBY server:<id> player_count 1
    GK-->>CL: player_id + server info

    CL->>GS: QUIC connect
    GS-->>CL: Connected event
    CL->>GS: GameMessage::Join
    GS-->>CL: GameMessage::Welcome

    loop every frame
        CL->>GS: PlayerInput (datagram)
        GS->>GS: Simulate + interest filter
        GS-->>CL: PositionBatch snapshots
    end

    loop every 5s
        GS->>OR: UDP Heartbeat JSON
        OR->>RD: HSET server:<id> fields + EXPIRE TTL
    end

    loop every scaler interval
        OR->>RD: list empty servers
        alt too few empties
            OR->>OR: spawn container via Docker
        else too many empties
            OR->>OR: stop/remove excess
        end
    end
```

---

## 4) Data Contracts and State Ownership

### Ownership boundaries
- Identity truth: Supabase (`PlayerInformation`).
- Fleet/server availability truth: Redis hashes `server:<id>` driven by orchestrator heartbeats.
- Live simulation truth: GameServer in-memory Bevy world.

### Temporary consistency mechanism
Gatekeeper increments `player_count` in Redis immediately after assignment. This bridges the delay until next heartbeat refresh from the game server.

---

## 5) Operational Notes

- Root compose runs core backend services and leaves client as local process.
- Orchestrator requires Docker socket mount to manage game-server containers.
- Heartbeat TTL allows passive cleanup of dead servers without explicit deregistration.
- QUIC transport handles both reliable messages and low-latency datagrams for gameplay.

---

## 6) Folder-to-Responsibility Summary

- `.cargo`: build profile tuning.
- `client`: UI/login + connection + gameplay rendering/input.
- `common`: cross-service protocol and Redis abstractions.
- `docs`: visual evidence of system behavior.
- `game_sockets`: protocol-agnostic networking layer.
- `GameServer`: authoritative simulation + session runtime + heartbeats.
- `gatekeeper`: auth and server assignment.
- `Orchestrator`: heartbeat ingestion and dynamic scaling.
- `target`: generated build artifacts.
