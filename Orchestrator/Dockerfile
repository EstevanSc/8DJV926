FROM rust:latest AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:trixie-slim

RUN apt-get update && apt-get install -y ca-certificates netcat-openbsd && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/orchestrator .
COPY mock_server.sh .
RUN chmod +x mock_server.sh

EXPOSE 8081

CMD ["./orchestrator"]