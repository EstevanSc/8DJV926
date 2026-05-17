# ==========================================================
# Stage 1: Build Environment
# ==========================================================
FROM rust:1.89-slim-bookworm AS builder

# Install system dependencies required to compile Bevy and network libraries
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    clang \
    lld \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the workspace configuration and source files from the root context
COPY game_sockets/ ./game_sockets/
COPY GameServer/ ./GameServer/

WORKDIR /app/GameServer

# Build the dedicated server target
RUN cargo build --release -p GameServer

# ==========================================================
# Stage 2: Runtime Environment
# ==========================================================
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the compiled binary from the builder stage
COPY --from=builder /app/GameServer/target/release/GameServer /app/GameServer
# Expose the default UDP game port
EXPOSE 7777/udp

# Default environment variables
ENV DS_PORT=7777
ENV DS_ZONE=zone_A
ENV MAX_PLAYERS=2
ENV ORCH_HOST=127.0.0.1:7000

CMD ["./GameServer"]