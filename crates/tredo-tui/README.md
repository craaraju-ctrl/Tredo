# tredo-tui

The primary Terminal UI for tredo — built with [ratatui](https://ratatui.rs/).

## What it provides

- **Chain-of-Thought Log** — Real-time scrollable log of every agent decision, color-coded by agent group
- **Portfolio Dashboard** — Current positions, P&L, equity, cash, win rate
- **Agent & Sub-Agent Tree** — Hierarchical tree view of all 16 sub-agents across 4 groups (Identifier, Verifier, Executer, Guardian) with:
  - Color-coded action badges per agent (🟢 PASS, 🔴 FAIL/HALT, 🟡 HOLD/SKIP, 🔵 START/UPDATED)
  - Skill score bars with direction icons (▲ Bullish, ▼ Bearish, ◆ Neutral) and confidence %
  - Live reasoning sub-lines for leaf agents
  - Unicode box-draw tree connectors (├──, └──, │)
- **Skill Consensus Header** — Aggregated skill signal at top of tree (net score, conviction, breakdown)
- **Color Legend** — Key showing all action badge colors and score symbols at bottom
- **Rules View** — Active `DisciplinedCore` rules with memory-adjusted values
- **Model Selection** — Browse and switch Ollama models interactively
- **Watchlist** — Live monitored symbols

## Usage

```bash
# From workspace root (backend must be running on port 8082):
cargo run -p tredo-tui

# Or via launcher:
./tredo tui
```

### Keyboard Controls

| Key | Action |
|-----|--------|
| `q` / `Ctrl-C` | Quit (backend keeps running) |
| `Tab` / `1`-`8` | Switch tabs |
| `↑` / `↓` | Scroll lists, scroll tree, select model |
| `Enter` | Switch model (in Models tab) |
| `r` | Force refresh |
| `Esc` | Reset scroll |

### API Endpoints Used

The TUI connects to the orchestrator at `http://localhost:8082/api`:

| Endpoint | Purpose |
|----------|---------|
| `/status` | Portfolio data (equity, cash, P&L, trades) |
| `/health` | System health (Kronos, LLM, loops) |
| `/cot` | Chain-of-thought entries |
| `/agents` | Agent tree JSON (hierarchy, roles) |
| `/skills` | Skill votes + aggregated signal |
| `/watchlist` | Monitored symbols |
| `/models` | Available Ollama models |
| `/models/set` | Switch active model |

This is the **primary interface** for the tredo system. Keyboard-driven for low-latency desk use.

Depends on `tredo-core` for types and data structures.
