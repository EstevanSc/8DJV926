# syntax=docker/dockerfile:1
# Multi-stage workspace build for the AI service.
# Build context must be the workspace root: dockerfile: ai_service/Dockerfile

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
RUN cargo chef cook --release -p ai_service --recipe-path recipe.json

# ── Stage 4: build the ai_service binary ──────────────────────────────────
COPY . .
RUN cargo build --release -p ai_service

# ── Stage 5: minimal runtime image ────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /workspace/target/release/ai_service /app/ai_service
COPY --from=builder /workspace/ai_service/config.toml /app/config.toml

ENV RUST_LOG=info

CMD ["/app/ai_service"]