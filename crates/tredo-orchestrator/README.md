# tredo-orchestrator

The autonomous brain of the tredo system — temporal loop driver and HTTP/WebSocket API server.

## What it provides

- **Temporal Loops** — Three-tier execution cadence:
  - Fast loop (5s): Price updates, SL/TP monitoring
  - Medium loop (5m): Full agent pipeline (Market Intelligence → Debate → Decision → Risk → Execution)
  - Slow loop (24h): Reflection and MetaControl rule adaptation
- **HTTP API** — Axum-based REST API for triggering cycles, querying COT, portfolio, and rules
- **WebSocket Server** — Real-time broadcast of COT entries, price updates, and system events
- **Shared State** — Manages the global `SharedState` across all agent groups

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Health check |
| POST | `/api/trigger_cycle` | Trigger a full med loop cycle |
| GET | `/api/cot` | Chain-of-thought log |
| GET | `/api/portfolio` | Current portfolio state |
| GET | `/api/rules` | Active discipline rules |
| GET | `/ws` | WebSocket for real-time updates |

## Usage

```bash
cargo run -p tredo-orchestrator
```

Depends on `tredo-core` and `tredo-autonomous`.
