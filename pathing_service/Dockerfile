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
COPY pathing_service ./pathing_service

# Build the pathing_service binary
RUN cargo build --release -p pathing_service

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/pathing_service /usr/local/bin/pathing_service

WORKDIR /app

# Set default environment variables
ENV RUST_LOG=info

ENTRYPOINT ["pathing_service"]
