# Orchestrator

Manages game server lifecycle and exposes server state to the Gatekeeper via Redis.

## Run

```bash
docker compose up
```

## Test

```bash
# Health check
curl.exe http://localhost:8080/api/health

# Redis state
docker exec -it orchestrator-redis redis-cli GET orchestrator:status
```

## Config

| Variable | Default |
|---|---|
| PORT | 8080 |
| REDIS_URL | redis://127.0.0.1:6379 |
| ENVIRONMENT | development |

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
INFO orchestrator: Successfully connected to Redis
INFO orchestrator: Redis ping successful: PONG
INFO orchestrator: Orchestrator status: online
INFO orchestrator: Server listening on 0.0.0.0:8080
```