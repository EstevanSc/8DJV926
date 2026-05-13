FROM rust:latest AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/orchestrator .

EXPOSE 8081

CMD ["./orchestrator"]