# tredo-tauri

Secondary desktop UI for tredo — built with [Tauri 2](https://v2.tauri.app/).

## What it provides

- Native desktop application (macOS, Linux, Windows)
- Vanilla JS frontend with pages: dashboard, COT view, portfolio, rules, settings
- Connects to the orchestrator API/WebSocket for live data
- Secondary interface — the primary UI is the ratatui Terminal UI

## Usage

```bash
# Build and run from workspace root:
cargo run -p tredo-tauri

# This requires Tauri system dependencies (webkit2gtk, etc.)
```

> **Note:** This is a secondary interface. The primary, feature-rich UI is `tredo-tui` (ratatui Terminal UI) which features hierarchical Agent & Sub-Agent Tree view with skill score bars, per-agent action badges, and a color legend.

Depends on `tredo-core` and `tredo-autonomous`.
