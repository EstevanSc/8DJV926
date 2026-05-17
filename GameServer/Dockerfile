# syntax=docker/dockerfile:1
# Multi-stage workspace build for the dedicated game server.
# Build context must be the workspace root: dockerfile: GameServer/Dockerfile

# ── Stage 1: cargo-chef base ──────────────────────────────────────────────
FROM rust:slim-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    cmake \
    clang \
    lld \
    git \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --locked
WORKDIR /workspace

# ── Stage 2: dependency planner ───────────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: dependency cache ─────────────────────────────────────────────
FROM chef AS builder
COPY --from=planner /workspace/recipe.json recipe.json
RUN cargo chef cook --release -p GameServer --recipe-path recipe.json

# ── Stage 4: build the GameServer binary ──────────────────────────────────
COPY . .
RUN cargo build --release -p GameServer

# ── Stage 5: minimal runtime image ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/target/release/GameServer /app/GameServer

# Defaults — overridden by docker-compose environment:
ENV DS_PORT=7777
ENV DS_ZONE=zone_A
ENV MAX_PLAYERS=2
# Use the Docker service name so the container can reach the orchestrator:
ENV ORCH_HOST=orchestrator:7000

# QUIC/UDP game port — clients connect here after receiving server info from gatekeeper
EXPOSE 7777/udp

CMD ["/app/GameServer"]
