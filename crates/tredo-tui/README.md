# tredo-tui

The primary Terminal UI for tredo — built with [ratatui](https://ratatui.rs/).

## What it provides

- **Chain-of-Thought Tree** — Real-time view of every agent decision, tagged with skills/rules/trained memory
- **Portfolio Dashboard** — Current positions, P&L, equity curve, drawdown
- **Rules View** — Active `DisciplinedCore` rules with memory-adjusted values
- **Agent Tree** — Agent hierarchy and execution status
- **Trading Desk** — Manual controls and trade journal
- **COT Timeline** — Scrollable history of all reasoning steps

## Usage

```bash
# From workspace root:
cargo run -p tredo-tui

# Or via launcher:
./tredo tui
```

This is the **primary interface** for the tredo system. Keyboard-driven for low-latency desk use.

Depends on `tredo-core` for types and data structures.
