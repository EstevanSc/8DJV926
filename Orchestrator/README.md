# Orchestrator

Manages game server lifecycle and exposes server state to the Gatekeeper via Redis.

## Setup

```bash
cp .env.example .env
```

Edit `.env` to configure the orchestrator. Key environment variables:

| Variable | Default | Description |
|---|---|---|
| ORCHESTRATOR_PORT | 8081 | HTTP API host port |
| ORCH_PORT | 7000 | UDP heartbeat listener host port |
| REDIS_PORT | 6379 | Redis host port (do not change) |
| DS_BINARY_PATH | ./dedicated_server | Path to dedicated server binary (use `./mock_server.sh` for testing) |
| DS_BASE_PORT | 7777 | Starting port for spawned servers |
| HOT_SERVERS_MIN | 1 | Minimum available servers to maintain |

## Run

```bash
docker compose up --build
```

## Test

### Health Check
```bash
curl.exe http://localhost:8081/api/health
```
Expected response: `{"status":"ok"}`

### Redis Server Registry
```bash
# List all registered servers
docker exec -it orchestrator-redis redis-cli KEYS "server:*"

# Get server details (replace <id> with server ID from above)
docker exec -it orchestrator-redis redis-cli HGETALL server:<id>

# Check TTL remaining for a server (in seconds)
docker exec -it orchestrator-redis redis-cli TTL server:<id>
```

## Config

All configuration is loaded from `.env` with environment variable overrides and compile-time defaults.

## Development

```bash
cargo fmt
cargo check
cargo clippy -- -D warnings
```

## Expected Logs

```bash
docker logs -f orchestrator-app
```

### Startup
```
INFO orchestrator: Starting orchestrator - environment: development, port: 8081, redis_url: redis://redis:6379
INFO orchestrator: Successfully connected to Redis
INFO orchestrator: Redis ping successful: PONG
INFO orchestrator: Orchestrator status: online
INFO orchestrator: Starting heartbeat listener task
INFO orchestrator: Starting scaler task
INFO orchestrator: Server listening on 0.0.0.0:8081
INFO orchestrator::services::heartbeat_listener: Heartbeat listener started on 0.0.0.0:7000
INFO orchestrator::services::scaler: Scaler: Need to spawn 2 servers
INFO orchestrator::services::scaler: Spawned dedicated server on port 7777 (PID: 20)
INFO orchestrator::services::scaler: Spawned dedicated server on port 7778 (PID: 21)
```

### Heartbeat Reception (every 5 seconds from each mock server)
```
INFO orchestrator::services::heartbeat_listener: Received heartbeat from 127.0.0.1:46677: 93 bytes
INFO orchestrator::services::heartbeat_listener: Updated server 7777 in Redis (status: available, TTL: 30s)
```

### Scaler Loop (every 5 seconds)
```
INFO orchestrator::services::scaler: Scaler: Need to spawn 2 servers
INFO orchestrator::services::scaler: Spawned dedicated server on port 7777 (PID: 20)
INFO orchestrator::services::scaler: Spawned dedicated server on port 7778 (PID: 21)
```