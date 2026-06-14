# Multi-stage production build for tredo (Trading Real-time Edge Decision Optimisation)
# Clean, current crates only. Paper trading focus. Services (Ollama + Kronos) are external/sidecars.
# Usage: docker build -t tredo . && docker run --rm -it tredo ./tredo-tui   (or ./tredo-orchestrator)
FROM rust:1.82 AS builder

WORKDIR /app

# Copy workspace manifests (include all active members for correct resolution)
COPY Cargo.toml Cargo.lock ./
COPY crates/tredo-core/Cargo.toml ./crates/tredo-core/
COPY crates/tredo-autonomous/Cargo.toml ./crates/tredo-autonomous/
COPY crates/tredo-orchestrator/Cargo.toml ./crates/tredo-orchestrator/
COPY crates/tredo-tui/Cargo.toml ./crates/tredo-tui/
COPY src-tauri/Cargo.toml ./src-tauri/
COPY crates/tredo-server/Cargo.toml ./crates/tredo-server/

# Dummy sources for dependency caching (all members that have lib/bin)
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

# Cache deps (note: tauri may need extra for full, but we build the important bins)
RUN cargo build --release -p tredo-core -p tredo-autonomous -p tredo-orchestrator -p tredo-tui || true

# Real source
COPY . .

# Final release builds (the important production pieces)
RUN cargo build --release -p tredo-orchestrator -p tredo-tui

# --- Runtime (slim, no heavy Tauri GUI deps unless you enable the secondary UI) ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the real production binaries
COPY --from=builder /app/target/release/tredo-orchestrator /app/tredo-orchestrator
COPY --from=builder /app/target/release/tredo-tui /app/tredo-tui

# Data dir for redb / episodes / state (mounted in real deploys)
RUN mkdir -p /app/data

ENV RUST_LOG=info
ENV PAPER_MODE=true   # Production default emphasis (enforced in docs + code comments)

# Orchestrator exposes API (if configured); TUI is terminal
EXPOSE 8080 8082

# Default to the primary beautiful TUI. Override with docker run ... /app/tredo-orchestrator
CMD ["./tredo-tui"]

# Notes for operators:
# - Start sidecars separately: Kronos (python) + Ollama before full autonomous quality.
# - Use the host ./tredo launcher for best local DX.
# - Mount volumes for persistent redb + history if desired.
# - This image is intentionally minimal. For the secondary Tauri UI you would need extra GUI libs (see old Dockerfile for reference).