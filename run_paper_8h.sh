#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# run_paper_8h.sh — 8-hour paper trading verification run
#
# Purpose: Run tredo in paper mode for 8 hours with detailed logging
# to verify the time-based exit and fast-path fixes improve trade
# frequency and prevent capital lock.
#
# Usage:
#   chmod +x run_paper_8h.sh
#   ./run_paper_8h.sh
#
# Output:
#   logs/paper_run_YYYYMMDD_HHMMSS/  —  run directory with all logs
#     tredo.log                         —  main system log (RUST_LOG=debug)
#     portfolio_snapshots.log           —  periodic portfolio state snapshots (every 5 min)
#     trade_events.log                  —  trade-specific events (executions, exits)
#     timings.log                       —  timing diagnostics for fast-path detection
#     env.txt                           —  snapshot of environment used
# ═══════════════════════════════════════════════════════════════════

set -euo pipefail

cd "$(dirname "$0")"

RUN_DIR="logs/paper_run_$(date +%Y%m%d_%H%M%S)"
mkdir -p "$RUN_DIR"

echo "╔══════════════════════════════════════════════════════════╗"
echo "║   tredo — 8-Hour Paper Mode Verification Run           ║"
echo "║   Start: $(date)                    ║"
echo "║   Logs:  $RUN_DIR                      ║"
echo "╚══════════════════════════════════════════════════════════╝"

# ── Environment ─────────────────────────────────────────────────
# Load env file if it exists
if [ -f config/tredo.env ]; then
    set -a; source config/tredo.env; set +a
fi

# Override with paper-mode-safe values
export LLM_PROVIDER=ollama
export LLM_ENDPOINT=http://localhost:11434
export ALPACA_PAPER=true
export PAPER_MODE=true
export INITIAL_BALANCE=100000
export WATCHLIST="${WATCHLIST:-BTC,ETH,SOL}"
export RUST_LOG="${RUST_LOG:-debug}"
export RUST_BACKTRACE=1

# Snapshot env for reproducibility
env | sort > "$RUN_DIR/env.txt"

echo ""
echo "📋 Configuration:"
echo "   Mode:      PAPER"
echo "   Symbols:   ${WATCHLIST}"
echo "   LLM:       ${LLM_PROVIDER} @ ${LLM_ENDPOINT}"
echo "   Log level: ${RUST_LOG}"
echo "   Duration:  8 hours (28800 seconds)"
echo ""

# ── Logging Setup ────────────────────────────────────────────────
# Main log: everything at debug level
exec > >(tee -a "$RUN_DIR/tredo.log") 2>&1

# ── Start the Run ────────────────────────────────────────────────
echo "[$(date +%H:%M:%S)] 🚀 Starting 8-hour paper run..."
echo "[$(date +%H:%M:%S)] Watch for: time-based exits (⏰ TIME EXIT), fast-path (skip_debate_si_llm)"
echo ""

START_TS=$(date +%s)
END_TS=$((START_TS + 28800))  # 8 hours

# Run the tredo binary in paper mode
cargo run -p tredo-runtime --bin tredo -- --mode paper 2>&1

EXIT_CODE=$?
END_TIME=$(date)

echo ""
echo "╔══════════════════════════════════════════════════════════╗"
echo "║   Run Complete                                          ║"
echo "║   End:   $END_TIME"
echo "║   Exit:  $EXIT_CODE"
echo "║   Logs:  $RUN_DIR/tredo.log"
echo "╚══════════════════════════════════════════════════════════╝"

# ── Post-Run Summary ────────────────────────────────────────────
echo ""
echo "📊 Post-Run Analysis:"
echo "   Time-based exits:"
grep -c "TIME EXIT" "$RUN_DIR/tredo.log" 2>/dev/null && echo "   found" || echo "   0"
echo ""
echo "   Fast-path decisions (skip_debate_si_llm):"
grep -c "skip_debate_si_llm" "$RUN_DIR/tredo.log" 2>/dev/null && echo "   found" || echo "   0"
echo ""
echo "   Total trades executed:"
grep -c "EXECUTED:" "$RUN_DIR/tredo.log" 2>/dev/null && echo "   counted" || echo "   0"
echo ""
echo "   SL/TP hits:"
grep -cE "STOP LOSS hit|TAKE PROFIT hit|TIME EXIT" "$RUN_DIR/tredo.log" 2>/dev/null || echo "   0"
echo ""

exit $EXIT_CODE
