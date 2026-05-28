# Build stage
FROM rust:latest AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY client ./client
COPY gatekeeper ./gatekeeper
COPY GameServer ./GameServer
COPY Orchestrator ./Orchestrator
COPY game_sockets ./game_sockets
COPY common ./common
COPY quadtree ./quadtree
COPY broker ./broker

# Build the quadtree binary
RUN cargo build --release -p quadtree

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/quadtree /usr/local/bin/quadtree

WORKDIR /app

# Set default environment variables
ENV QUADTREE_WORLD_SIZE=100.0
ENV QUADTREE_MAX_CAPACITY=4
ENV QUADTREE_MAX_DEPTH=10
ENV QUADTREE_NEARBY_MARGIN=5.0
ENV QUADTREE_LISTEN_PORT=5000
ENV QUADTREE_ORCHESTRATOR_HOST=localhost
ENV QUADTREE_ORCHESTRATOR_PORT=5000
ENV QUADTREE_ENTITY_ADD_INTERVAL_MS=1000
ENV RUST_LOG=info

ENTRYPOINT ["quadtree"]
