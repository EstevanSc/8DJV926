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
COPY AbilityService ./AbilityService

# Build the pathing_service binary
RUN cargo build --release -p AbilityService

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/AbilityService /usr/local/bin/AbilityService

WORKDIR /app

# Set default environment variables
ENV RUST_LOG=info

ENTRYPOINT ["AbilityService"]