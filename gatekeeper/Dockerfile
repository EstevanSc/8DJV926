# syntax=docker/dockerfile:1
# Multi-stage build for the gatekeeper service.
# Exposes port 3000 (HTTP REST /login) and 3001/udp (QUIC for client game sessions).

# ── Stage 1: cargo-chef base ──────────────────────────────────────────────
FROM rust:slim-bookworm AS chef
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    cmake \
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
RUN cargo chef cook --release -p gatekeeper --recipe-path recipe.json

# ── Stage 4: build the gatekeeper binary ──────────────────────────────────
COPY . .
RUN cargo build --release -p gatekeeper

# ── Stage 5: minimal runtime image ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/target/release/gatekeeper /usr/local/bin/gatekeeper

ENV REDIS_URL="redis://redis:6379"

# HTTP REST endpoint (POST /login, GET /health)
EXPOSE 3000
# QUIC endpoint — clients connect here after login to establish a game session
EXPOSE 3001/udp

CMD ["gatekeeper"]
