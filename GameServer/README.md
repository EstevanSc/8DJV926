# Game Dedicated Server

Dedicated game server built with the **Bevy Engine** utilizing **QUIC (UDP)** via `game_sockets` for reliable/unreliable state synchronization.
## Architecture & Communication Flow

* **Game Traffic:** Clients connect via standard **QUIC over UDP** (Default: Port `7777`).
* **Orchestration:** The server emits outward **JSON over UDP heartbeats** to register its capacity and active player states with the cluster management service.

---

## Configuration Variables

The server dynamically configures itself via system environment variables. You can override these in your host shell or within your `docker-compose.yml` file:

| Variable | Description | Default Value |
| --- | --- | --- |
| `DS_PORT` | The UDP port the server listens on for game clients | `7777` |
| `DS_ZONE` | Identifier for the localized deployment cluster | `zone_A` |
| `MAX_PLAYERS` | Capacity boundary threshold before moving to a `FULL` state | `2` |
| `ORCH_HOST` | Target IP/Domain and Port of the central supervisor/orchestrator | `127.0.0.1:7000` |

---

## Getting Started

### Prerequisites

* [Docker](https://www.docker.com/) and [Docker Compose](https://docs.docker.com/compose/)
* [Rust toolchain](https://rustup.rs/) (if compiling/running binaries locally)

### 1. Run the Server (Via Docker)

To build the optimized release layer and spin up the dedicated container isolated in the background:

```bash
# Navigate to the workspace sub-directory if executing directly
cd GameServer

# Boot up the server 
docker-compose up -d --build
```

To watch live gameplay connections and state engine logs:

```bash
docker-compose logs -f game-server
```

### 2. Run the Mock Client (Locally)

A barebones validation client is bundled to simulate handshakes, serializing/deserializing network payloads, and standard state interaction.

Run it directly from the workspace root:

```bash
cargo run --bin test_client
```

---

## Troubleshooting & Network Caveats

### "Client cannot connect to the Docker container"

If the server is running inside Docker but your mock client on the host machine times out trying to connect:

1. Ensure you have mapped the port explicitly as UDP in your configuration. Standard TCP mapping (`7777:7777`) will reject QUIC handshakes. It must be explicitly bound as `7777:7777/udp`.
2. Verify that no local firewall (e.g., UFW, Windows Defender) is blocking incoming UDP traffic on that port.

### "Server heartbeats are not reaching my Orchestrator"

If your orchestrator is running natively on your host machine (outside Docker) and the game server is inside Docker, setting `ORCH_HOST=127.0.0.1:7000` will fail because `127.0.0.1` inside a container resolves to itself.