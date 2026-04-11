# --- Build stage ---
FROM rust:1.87-slim AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY contracts/ contracts/
COPY src/ src/

# Build release binary
RUN cargo build --release

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/neurust /usr/local/bin/neurust
COPY config/ /etc/neurust/config/
COPY data/ /etc/neurust/data/

ENV NEURUST_CONFIG=/etc/neurust/config/neurust.yml

EXPOSE 8080

CMD ["neurust"]
