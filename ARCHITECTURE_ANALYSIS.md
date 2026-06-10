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
- `quadtree`: Spacial service dictating the server configuration and dealing with authority transfers
- `broker`: The message broker, which is responsible for receiving messages and forwarding them to the appropriate receivers.

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
   - receives target server (`ip`, `port`, `zone`).
   - stores `GameSession` resource and transitions to `Connecting`.
4. Connecting flow:
   - creates `GamePeer` with QUIC backend.
   - connects to server.
5. In-game flow:
   - `input.rs`: reads keyboard, serializes `PlayerInput`, sends on stream 0.
   - `net.rs`: receives the individual position of every entity in its area of interest.
   - `interpolation.rs`: creates circles/name tags for unseen entity ids, then lerps transform to latest target position.

### Key design choices
- Uses shared packet types from `common` (`PlayerInput`), reducing wire-protocol drift.
- Uses optimistic interpolation with `POSITION_DELTA_THRESHOLD` to avoid micro-jitter updates.
- Keeps network peer in `Mutex<GamePeer>` for Bevy resource synchronization.

---

## `common`

### Purpose
Shared contract crate used by gatekeeper, orchestrator, server, and client.

### Contents and role
- `packets.rs`:
  - Defines network schemas for QUIC/UDP, including `PositionSnapshot` and `PositionBatch` for entity sync, plus `ConnectRequest` and `AuthAck` for session handshakes.
- `heartbeat.rs`:
  - Contains the `Heartbeat` schema sent from game servers to the Orchestrator for tracking server status.
- `server_info.rs`:
  - Provides the canonical `ServerInfo` structure for server metadata (`id`, `ip`, `port`, `zone`, `status`, `player_count`, `max_players`), used by the Gatekeeper for routing.
- `redis_client.rs`:
  - Implements an async wrapper over Redis `ConnectionManager` with helpers for `scan`, `hset_multiple`, `hget`, `expire`, and `hincr`.
- `constants.rs`:
  - Stores transport and visibility tuning values such as `POSITION_DELTA_THRESHOLD` and `INTEREST_RADIUS_TILES`.
- `redis_keys.rs`:
  - Defines shared key naming patterns like `server:<id>` and `servers:active`.
- `shard_data.rs`:
  - Contains shared structures for quadtree management, including `Boundary` logic, quadrant subdivision, and serialization.
- `topics.rs` & `broker_messages.rs`:
  - Defines the messaging contract for the broker, including `Publish`/`Subscribe` types and a strongly-typed topic system for inter-service communication.

### Why it matters
This crate is the integration backbone. It aligns data models across services, centralizes serialization logic, and avoids duplicate schema definitions.
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
Dedicated authoritative simulation server for gameplay sessions and shard-based entity management.

### Structure
- `src/main.rs`: Starts Bevy app with `ServerPlugin` + `SimulationPlugin`.
- `src/server.rs`: Network bind, broker event loop, message handling (ownership/input), and heartbeat sending.
- `src/simulation.rs`: Physics world, entity lifecycle (spawn/despawn), ownership transition logic, and snapshot broadcasting.
- `src/char_controller.rs`: Character movement, damping, and grounded-state systems.
- `src/net.rs`: Inter-thread simulation command channel and connection registry helpers.
- `src/messages.rs`: `GameMessage::{Join,Welcome}` legacy authentication contract.
- `src/heartbeat.rs`: Local heartbeat struct used for reporting status to the Orchestrator.

### Runtime behavior
1. On startup:
   - Reads env config (e.g., `DS_PORT`, `DS_SHARD_CENTER_X`, `DS_ZONE`, `MAX_PLAYERS`).
   - Binds QUIC listener and establishes control/snapshot streams with the broker.
   - Announces shard boundary creation via the broker.
2. Broker Event Handling:
   - Processes `ClaimOwnership` to promote ghosts to local players and `ReleaseOwnership` to demote them.
   - Routes player inputs received from the broker into the simulation.
3. Simulation Tick:
   - Drains join/leave/input commands from the inter-thread channel.
   - Updates physics and enforces boundary logic to despawn entities exiting the shard.
   - Publishes authoritative `PositionPayload` to the broker for active entities.
4. Heartbeat:
   - Every 5 seconds sends JSON UDP heartbeat to the orchestrator reporting server occupancy and status.

### Notable architecture detail
The game server operates within a distributed quadtree; entities are marked as "Local" (full physics authority) or "Ghost" (network-synchronized). Authority is dynamically handed off between shards via broker publications when players cross defined `Boundary` limits.
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
   - broker connection info (`ip`, `port`) for client to connect.

### Key integration role
Acts as traffic director from identity layer to runtime world instance.

---

## `Orchestrator`

### Purpose
Fleet-control plane responsible for the lifecycle management and health monitoring of game server instances.

### Structure
- `src/main.rs`: Entry point; initializes the configuration, Redis connection, and background worker tasks.
- `src/config.rs`: Environment variable parsing and default configuration management.
- `src/services/heartbeat_listener.rs`: UDP background task for ingesting heartbeats and maintaining Redis-based server states.
- `src/services/shard_handler.rs`: Logic for processing quadtree layout updates and orchestrating server spawning/teardown.
- `src/docker_ops.rs`: Docker daemon abstraction (via `bollard`) for container lifecycle (spawn/stop).
- `src/quic_server.rs`: QUIC listener for receiving real-time spatial shard updates from the Quadtree service.
- `src/api/health.rs`: Health check endpoint for service liveness verification.

### Runtime behavior
1. **Startup**: 
   - Loads configuration (`PORT`, `REDIS_URL`, `DS_BASE_PORT`) and establishes Redis connectivity.
   - Spawns the background heartbeat listener (UDP) and the shard handler task.
2. **Heartbeat Listener**:
   - Ingests JSON UDP payloads from game servers.
   - Updates server status (`empty`, `available`, `full`) in Redis with a TTL (`HEARTBEAT_TTL_SECONDS`) to handle node failures.
3. **Shard & Quadtree Updates**:
   - Receives boundary updates via QUIC from the Quadtree service.
   - Compares the desired state with the current server layout and triggers Docker operations to add or remove instances accordingly.
4. **Docker Management**:
   - Spawns containers with required environment variables (`DS_ID`, `DS_PORT`, `DS_SHARD_CENTER_X/Y`, etc.).
   - Maps dedicated game ports to the host and performs pre-registration in Redis with `status=starting` before the first heartbeat confirms the server is ready.

### Why this is important
The Orchestrator acts as the glue between spatial partitioning (Quadtree) and physical deployment (Docker). By reacting dynamically to shard updates, it ensures the fleet maintains the necessary capacity for the current game world topology while handling the full lifecycle of ephemeral server instances.
---
### `QUADTREE`

---

## `quadtree`

### Purpose
The `quadtree` crate serves as the spatial partitioning coordinator and dynamic world sharding engine. It is responsible for tracking entity positions globally, dynamically splitting or merging spatial regions (shards) based on player density, and informing the `Orchestrator` and `broker` of world boundary updates to facilitate horizontal scaling of game server instances.

### Structure
- `config.rs`: Manages environment configuration mapping parameters such as world dimensions, depth limitations, and capacity rules.
- `quic_client.rs`: A wrapper layer over `game_sockets` providing dedicated asynchronous QUIC client connection logic optimized for communication with the central orchestrator and broker.
- `main.rs`: Orchestrates the main execution tick loop, handles network events, runs spatial subdivision geometry logic, and coordinates player placement and area-of-interest management.

### Internal Behavior
1. **Startup & Network Initialization**:
   - Spawns and maps configuration values derived from environment variables via `Config::from_env()`.
   - Establishes two independent, parallel QUIC links using `QuicClient`: one dedicated to the `Orchestrator` for fleet topology reporting, and one to the `broker` for real-time messaging.
   - Registers its connection with the broker, announcing its system type (`SendingSystem::Quadtree`), and subscribes to structural topics: `Topic::ShardCreated` and `Topic::PlayerStartingPosition`.
2. **The Tick Loop**:
   - Executes periodically according to the interval configured via `entity_add_interval_ms`.
   - On each tick, it processes incoming QUIC messages from the broker, resolves player placement, maps field-of-interest sets, and determines if a tree structural mutation is necessary.
3. **Dynamic Rebuild & Subdivisions**:
   - When entity density within a local spatial sector violates `max_capacity`, the quadtree triggers a geometry calculation. It splits the node into four sub-quadrants (`NorthEast`, `NorthWest`, `SouthEast`, `SouthWest`) provided the tree hasn't breached its `max_depth` restriction.
   - Structural updates execute safely by flushing positions and executing an atomic tree rewrite. Leaf boundaries collected during the query update the system's runtime collection (`SharedShardSet` and `SharedShardMap`).
4. **Player Spawning & Lifecycle Handshakes**:
   - When a `PlayerStartingPosition` payload is pulled from the broker queue, the system maps the coordinates to its active spatial cells using `find_shard_for_position`.
   - If a valid, initialized Shard UUID exists for that boundary, the quadtree executes a cluster-wide lifecycle setup via the broker:
     - Subscribes the assigned target `GameServer` (shard) to the player's inputs, disconnect events, and position channels.
     - Subscribes the quadtree itself to the entity's movement updates.
     - Publishes a `PlayerStartingPositionInShard` notification to instruct the game server to spawn the actor.
5. **Area of Interest (AoI) Culling**:
   - Leverages `area_of_interest_radius` and a configured `nearby_margin` to dynamically evaluate entity proximity.
   - Identifies which entities are close enough to cross shard borders, ensuring that position states are "ghosted" or mirrored across neighboring shard boundaries to guarantee smooth visual continuity for clients moving near edge lines.

### Key Design Choices
- **Dual QUIC Separation**: Separating broker messaging from orchestrator fleet reporting allows telemetry collection and low-latency state publishing to happen on separate streams without resource contention.
- **Safe Rebuild Synchronization**: The `rebuild` sequence utilizes structured memory take swaps (`mem::take`) and explicitly protects data mappings from premature states during insertion cycles. It preserves existing active Shard UUID connections during rewrites so the Orchestrator doesn't clean up running containers before replacements initialize.
- **Deferred Deletions**: Tracked via `PendingShardToDestroy` structures, old shards are gracefully retained until newly substituted spaces register themselves over the network, eliminating coordinate gaps or orphan player drops during live splits.

---

## `Broker`

### Purpose
Centralized message-routing service that acts as the pub/sub backbone for inter-service communication across the distributed game world.

### Structure
- `src/main.rs`: Initializes the broker network peer and starts the main event loop.
- `src/net.rs`: Implements the `BrokerState` logic, including connection management, topic subscription tracking, and message broadcasting.

### Runtime behavior
1. **Startup**: 
   - Parses `BROKER_PORT` from environment variables.
   - Binds a `GamePeer` using the QUIC backend to listen for incoming connections.
2. **Connection Management**: 
   - Tracks active `GameConnection` instances and maintains an internal `connection_map` linking UUIDs to network peers.
   - Automatically handles cleanup of subscriptions and state when a client disconnects.
3. **Pub/Sub Logic**:
   - `Subscribe`/`Unsubscribe`: Manages a registry of subscribers per topic, allowing services to express interest in specific events like `Input`, `EntityPositionUpdate`, or `Disconnect`.
   - `Publish`: Receives payloads from publishers, identifies interested subscribers, and forwards the data as a `Broadcast` message over reliable streams.
   - `Connect`: Allows systems (Client, Server, Orchestrator) to register their system identity with the broker.

### Why this is important
The broker decouples services by eliminating the need for point-to-point communication. It enables a dynamic architecture where game servers can subscribe to entity-specific topics (e.g., input from a specific player) or system-wide events (e.g., shard updates) without needing to know the location or address of the originating service.

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