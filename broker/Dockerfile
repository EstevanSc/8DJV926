# syntax=docker/dockerfile:1
# Multi-stage workspace build for the MMO Broker.
# Build context must be the workspace root: dockerfile: broker/Dockerfile

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
# Build ONLY the dependencies required by the broker package (-p broker)
RUN cargo chef cook --release -p broker --recipe-path recipe.json

# ── Stage 4: build the broker binary ──────────────────────────────────────
COPY . .
RUN cargo build --release -p broker

# ── Stage 5: minimal runtime image ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/target/release/broker /app/broker

# Defaults — overridden by docker-compose environment:
ENV BROKER_PORT=7777
ENV BROKER_PUBLIC_IP=localhost

# QUIC/UDP port for client/shard connections
EXPOSE 7777/udp

CMD ["/app/broker"]