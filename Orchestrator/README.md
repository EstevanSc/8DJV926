# Orchestrator

Manages game server lifecycle and exposes server state to the Gatekeeper via Redis.

## Setup

```bash
cp .env.example .env
```

`.env` configures host ports and environment. Edit it to avoid conflicts with other services (e.g. Gatekeeper on 8080).

## Run

```bash
docker compose up
```

## Test

```bash
# Health check
curl.exe http://localhost:8081/api/health

# Redis state
docker exec -it orchestrator-redis redis-cli GET orchestrator:status
```

## Config

| Variable | Default | Description |
|---|---|---|
| ORCHESTRATOR_PORT | 8081 | Host port for the orchestrator |
| REDIS_PORT | 6379 | Host port for Redis (do not change) |
| ENVIRONMENT | development | Runtime environment |

## Development

```bash
cargo fmt
cargo check
cargo clippy -- -D warnings
```

## Expected logs

```bash
docker logs -f orchestrator-app
```

```
INFO orchestrator: Starting orchestrator - environment: development, port: 8081, redis_url: redis://redis:6379
INFO orchestrator: Successfully connected to Redis
INFO orchestrator: Redis ping successful: PONG
INFO orchestrator: Orchestrator status: online
INFO orchestrator: Server listening on 0.0.0.0:8081
```