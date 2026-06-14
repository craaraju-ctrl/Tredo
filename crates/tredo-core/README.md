# tredo-core

Foundation crate for the tredo autonomous trading system.

## What it provides

- **DisciplinedCore** — Hard Rust gates for professional trading rules (pivots, trend, confluence, position sizing, drawdown, session rules)
- **Memory** — redb-based hot state, vector memory for semantic recall, agentmemory client for cross-session intelligence
- **LLM Client** — Async executor for Ollama (primary), fallback for OpenAI/Claude
- **Kronos Client** — Rust client for the Kronos time-series forecast sidecar with graceful fallback
- **AgentSkill Trait** — Pluggable deterministic capability trait (`name`, `execute`, `is_available`)
- **Paper Engine** — Realistic paper execution with slippage, position sizing, and full trade journaling
- **Pattern Detection** — 15 candlestick pattern detectors with multi-timeframe confirmation
- **Types** — Shared types for episodes, market context, decisions, skills, and configuration

## Usage

```rust
use tredo_core::DisciplinedCore;
use tredo_core::skills::AgentSkill;
```

This crate is a dependency of `tredo-autonomous`, `tredo-orchestrator`, and `tredo-tui`.
