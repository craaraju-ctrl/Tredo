# tredo-server

Optional production HTTP server for the tredo trading system.

## What it provides

- Lightweight HTTP API server for production deployments
- Axum-based with WebSocket support
- Exposes portfolio, rules, and system status endpoints
- Designed for headless or remote operation

## Usage

```bash
cargo run -p tredo-server
```

Depends on `tredo-core`.

> **Note:** For local development and the full TUI experience, use `tredo-orchestrator` or `./tredo tui` instead.
