# tredo-tui

The primary Terminal UI for tredo — built with [ratatui](https://ratatui.rs/).

## What it provides

- **Chain-of-Thought Log** — Real-time scrollable log of every agent decision, color-coded by agent group
- **Portfolio Dashboard** — Current positions, P&L, equity, cash, win rate
- **Agent & Sub-Agent Tree** — Hierarchical tree view of all sub-agents across 4 groups (Identifier, Verifier, Executer, Guardian) with:
  - Color-coded action badges per agent (🟢 PASS, 🔴 FAIL/HALT, 🟡 HOLD/SKIP, 🔵 START/UPDATED)
  - Skill score bars with direction icons (▲ Bullish, ▼ Bearish, ◆ Neutral) and confidence %
  - Live reasoning sub-lines for leaf agents
  - Unicode box-draw tree connectors (├──, └──, │)
- **Broker & Data** — Real-time status of all configured brokers (Alpaca, Zerodha, Binance), connection state, account balances, margin usage, and active orders
- **Settings & Control** — Interactive system configuration panel with Models, Agents, Skills, and Risk parameter editing. Toggle agents on/off, adjust risk parameters (+/-), confirm changes with y/N
- **Color Legend** — Key showing all action badge colors and score symbols at bottom
- **Rules View** — Active `DisciplinedCore` rules with memory-adjusted values
- **Model Selection** — Browse and switch Ollama models interactively
- **Watchlist** — Live monitored symbols
- **Backtest View** — Run and view backtest results with equity curve and metrics
- **Health Dashboard** — Real-time system health (Kronos, LLM, loops, brokers)
- **Performance View** — Strategy performance metrics (Sharpe, win rate, max drawdown, etc.)
- **Positions Panel** — Detailed open positions with unrealized P&L
- **Policy Cache View** — View learned policy cache entries and hit rates
- **Scanner** — Market scanner with filtering and sorting
- **WebSocket Client** — Real-time WS connection to orchestrator for live updates

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
| `Tab` / `1`-`0` | Switch tabs |
| `↑` / `↓` | Scroll lists, scroll tree, select model |
| `←` / `→` | Navigate action buttons |
| `Enter` | Activate / Confirm |
| `b` | Jump to Broker page |
| `S` | Jump to Settings page |
| `r` | Force refresh |
| `s` | Sort current table by next column |
| `/` | Search/filter (COT Log, Policy Cache) |
| `?` | Toggle keyboard shortcuts overlay |
| `Esc` | Back / Close / Cancel |

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
| `/backtest` | Backtest results |
| `/performance` | Strategy metrics |
| `/ws` | WebSocket for real-time updates |

This is the **primary interface** for the tredo system. Keyboard-driven for low-latency desk use.

Depends on `tredo-core` for types and data structures.
