# syntax=docker/dockerfile:1
# Multi-stage workspace build for the orchestrator service.
# Build context must be the workspace root: dockerfile: Orchestrator/Dockerfile

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
RUN cargo chef cook --release -p orchestrator --recipe-path recipe.json

# ── Stage 4: build the orchestrator binary ────────────────────────────────
COPY . .
RUN cargo build --release -p orchestrator

# ── Stage 5: minimal runtime image ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates netcat-openbsd \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/target/release/orchestrator /usr/local/bin/orchestrator
COPY --from=builder /workspace/Orchestrator/mock_server.sh /app/mock_server.sh
RUN sed -i 's/\r//' /app/mock_server.sh && chmod +x /app/mock_server.sh

ENV PORT=8081
ENV ORCH_PORT=7000

# HTTP fleet-management API
EXPOSE 8081
# UDP heartbeat listener (game servers → orchestrator)
EXPOSE 7000/udp

CMD ["orchestrator"]
