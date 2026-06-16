# tredo-server

Production HTTP server for the tredo trading system.

## What it provides

- **Production HTTP API** — Axum-based server with WebSocket support for remote and headless operation
- **Broker Registry** — `BrokerRegistry` that dispatches to `PaperBroker` or live broker adapters (Alpaca, Zerodha) based on mode
- **Mode Switching** — Dynamic switching between `paper` and `live` modes via API
- **Portfolio & Trade APIs** — Full REST endpoints for positions, trades, orders, and account summary
- **Broker Configuration** — `POST /api/broker/config` and `POST /api/broker/test` for live broker setup
- **WebSocket Broadcast** — Real-time price updates and trade events via WS
- **Static File Serving** — Serves the Tauri frontend SPA for browser access

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/summary` | Portfolio summary (current mode) |
| GET | `/api/positions` | Open positions |
| GET | `/api/trades` | Recent trade history |
| POST | `/api/trade` | Place an order |
| POST | `/api/close` | Close a position |
| POST | `/api/price` | Update market price for a symbol |
| POST | `/api/reset` | Reset paper portfolio |
| GET | `/api/mode` | Get current mode (paper/live) |
| POST | `/api/mode` | Switch mode |
| POST | `/api/broker/config` | Update broker API config |
| POST | `/api/broker/test` | Test broker connection |
| WS | `/ws` | Real-time updates |

## Usage

```bash
cargo run -p tredo-server -- --port 8080
```

Depends on `tredo-core`, `tredo-autonomous`, and `tredo-runtime` (for broker plugin system).

> **Note:** For local development and the full TUI experience, use `tredo-orchestrator` or `./tredo tui` instead. This server is designed for production headless deployment.
