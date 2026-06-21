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
COPY ai_service ./ai_service
COPY database_service ./database_service

# Build the database_service binary
RUN cargo build --release -p database_service

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/database_service /usr/local/bin/database_service

WORKDIR /app

# Set default environment variables
ENV RUST_LOG=info

ENTRYPOINT ["database_service"]
