# Multi-stage production build for tredo (Trading Real-time Edge Decision Optimisation)
# Usage: docker build -t tredo . && docker run --rm -it tredo ./tredo-tui (or ./tredo-orchestrator)
#
# Services (Ollama + Kronos) run as external sidecars.
# Requires BuildKit: DOCKER_BUILDKIT=1 docker build ...

# ── Builder Stage ────────────────────────────────────────────────
FROM rust:slim-bookworm AS builder

WORKDIR /app

# Copy workspace manifests for dependency caching
COPY Cargo.lock Cargo.toml ./
COPY crates/tredo-core/Cargo.toml ./crates/tredo-core/
COPY crates/tredo-autonomous/Cargo.toml ./crates/tredo-autonomous/
COPY crates/tredo-orchestrator/Cargo.toml ./crates/tredo-orchestrator/
COPY crates/tredo-tui/Cargo.toml ./crates/tredo-tui/
COPY src-tauri/Cargo.toml ./src-tauri/
COPY crates/tredo-server/Cargo.toml ./crates/tredo-server/

# Dummy sources to cache dependency compilation
RUN mkdir -p \
    crates/tredo-core/src \
    crates/tredo-autonomous/src \
    crates/tredo-orchestrator/src \
    crates/tredo-tui/src \
    src-tauri/src \
    crates/tredo-server/src && \
    echo "fn main(){}" > crates/tredo-core/src/lib.rs && \
    echo "fn main(){}" > crates/tredo-autonomous/src/lib.rs && \
    echo "fn main(){}" > crates/tredo-orchestrator/src/main.rs && \
    echo "fn main(){}" > crates/tredo-tui/src/main.rs && \
    echo "fn main(){}" > src-tauri/src/main.rs && \
    echo "fn main(){}" > crates/tredo-server/src/main.rs

# Cache and build dependencies (layer is reused when Cargo.lock is unchanged)
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release -p tredo-core -p tredo-autonomous -p tredo-orchestrator -p tredo-tui

# Copy real source and build final binaries
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release -p tredo-orchestrator -p tredo-tui && \
    cp /app/target/release/tredo-orchestrator /app/tredo-orchestrator && \
    cp /app/target/release/tredo-tui /app/tredo-tui

# ── Runtime Stage ────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/tredo-orchestrator /app/tredo-orchestrator
COPY --from=builder /app/tredo-tui /app/tredo-tui

RUN mkdir -p /app/data

ENV RUST_LOG=info
ENV PAPER_MODE=true

EXPOSE 8080 8082

CMD ["./tredo-tui"]
