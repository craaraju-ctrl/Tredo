# tredo-orchestrator

The autonomous brain of the tredo system — temporal loop driver and HTTP/WebSocket API server.

## What it provides

- **Temporal Loops** — Three-tier execution cadence:
  - Fast loop (5s): Price updates, SL/TP monitoring
  - Medium loop (5m): Full agent pipeline (Market Intelligence → Debate → Decision → Risk → Execution) with per-sub-agent COT entry pushing
  - Slow loop (24h): Reflection and MetaControl rule adaptation
- **HTTP API** — Axum-based REST API for triggering cycles, querying COT, portfolio, rules, agent tree, skill scores, backtest, health, and performance
- **Agent Tree JSON** — Full `AutonomousOrchestrator::tree_json()` hierarchy (4 groups, sub-agents with roles)
- **Per-Sub-Agent COT** — Each pipeline cycle pushes COT entries for every sub-agent with action badges, confidence, and reasoning
- **Skill Scores API** — Real-time skill votes and aggregated signal from MarketIntelligence
- **WebSocket Server** — Real-time broadcast of COT entries, price updates, and system events
- **Shared State** — Manages the global `SharedState` across all agent groups
- **Loop Manager** — Dynamic start/stop of background temporal loops with graceful shutdown

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Health check (Kronos, Loops, LLM status) |
| GET | `/api/status` | Portfolio summary (equity, cash, P&L) |
| POST | `/api/trigger_cycle` | Trigger a full medium loop cycle |
| GET | `/api/cot` | Chain-of-thought log (all entries) |
| GET | `/api/agents` | Agent hierarchy tree JSON |
| GET | `/api/skills` | Skill votes + aggregated signal |
| GET | `/api/watchlist` | Current watchlist symbols |
| POST | `/api/watchlist/add` | Add symbol to watchlist |
| POST | `/api/watchlist/remove` | Remove symbol from watchlist |
| GET | `/api/models` | Available Ollama models |
| POST | `/api/models/set` | Switch active LLM model |
| POST | `/api/start` | Start autonomous loops |
| POST | `/api/stop` | Stop autonomous loops |
| POST | `/api/trade` | Manual paper trade execution |
| POST | `/api/rules` | Update discipline rules |
| GET | `/api/backtest` | Run backtest simulation |
| GET | `/api/price` | Live stock price |
| GET | `/api/crypto/exchanges` | Available crypto exchanges |
| GET | `/api/crypto/symbols` | Tradable crypto symbols |
| GET | `/api/crypto/prices` | Live crypto prices (multi-exchange) |
| GET | `/api/crypto/market` | 24h crypto market stats |
| GET | `/api/news` | Latest news headlines |
| GET | `/api/performance` | Performance metrics (Sharpe, win rate, etc.) |
| GET | `/api/health/detailed` | Detailed health check with all subsystems |
| GET | `/ws` | WebSocket for real-time updates |

## Usage

```bash
# Default port 8080, or set PORT=8082
PORT=8082 cargo run -p tredo-orchestrator
```

Depends on `tredo-core` and `tredo-autonomous`.
