# Multi-stage build for a small runtime image

FROM rust:latest AS builder
WORKDIR /app

# Leverage Docker layer caching for dependencies
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/arbitrage-detector /usr/local/bin/arbitrage-detector

# Run as non-root
RUN useradd -m appuser
USER appuser

ENV RUST_LOG=info

# Use shell form to handle environment variables properly
CMD arbitrage-detector


