#!/usr/bin/env bash
#
# start_orchestrator.sh — Build (if stale), load environment, and launch the
#                         tredo orchestrator.
#
# Usage:  ./start_orchestrator.sh
#         LOG_DIR=/var/log/tredo ./start_orchestrator.sh   (custom log path)
#
# The script always runs `cargo build --release` first.  Cargo is incremental,
# so on repeated invocations it finishes in <1 s when nothing changed.
#
# Designed to be invoked by launchd (macOS) or systemd (Linux).
# On its own it runs in the foreground; redirect stdout/stderr as needed.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

ENV_FILE="$PROJECT_DIR/config/tredo.env"
LOG_DIR="${LOG_DIR:-/tmp/tredo-logs}"

# ── Build (release) if binary is missing or source has changed ──────────────
#
# We run the build *before* sourcing the env file so that environment variables
# intended for the orchestrator don't leak into the compiler process.
#
BINARY="$PROJECT_DIR/target/release/tredo-orchestrator"

echo "[orchestrator] Rebuilding release binary if stale…" >&2
cd "$PROJECT_DIR"
cargo build --release 2>&1

echo "[orchestrator] Build complete (or up-to-date)." >&2

# ── Sanity checks ──────────────────────────────────────────────────────────
if [ ! -f "$BINARY" ]; then
    echo "FATAL: Binary still missing after build: $BINARY"
    exit 1
fi
if [ ! -f "$ENV_FILE" ]; then
    echo "FATAL: Env file not found: $ENV_FILE"
    exit 1
fi

mkdir -p "$LOG_DIR"

# ── Load environment variables ──────────────────────────────────────────────
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

# ── Start the orchestrator ─────────────────────────────────────────────────
cd "$PROJECT_DIR"
exec "$BINARY" >> "$LOG_DIR/orchestrator.log" 2>> "$LOG_DIR/orchestrator.err"
