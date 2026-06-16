# tredo-core

Foundation crate for the tredo autonomous trading system.

## What it provides

- **DisciplinedCore** — Hard Rust gates for professional trading rules (pivots, trend, confluence, position sizing, drawdown, session rules)
- **Memory** — redb-based hot state, vector memory for semantic recall, agentmemory client for cross-session intelligence
- **LLM Client** — Async executor for Ollama (primary), fallback for OpenAI/Claude
- **Kronos Client** — Rust client for the Kronos time-series forecast sidecar with graceful fallback
- **AgentSkill Trait** — Pluggable deterministic capability trait (`name`, `execute`, `is_available`)
- **BrokerAdapter Trait** — Unified interface for paper and live broker adapters (`connect`, `place_order`, `get_summary`, `get_positions`, etc.)
- **Paper Engine** — Realistic paper execution with slippage, position sizing, and full trade journaling
- **Backtest Engine** — CSV-driven backtest with realistic fills and performance tracking
- **Pattern Detection** — 15 candlestick pattern detectors with multi-timeframe confirmation
- **Types** — Shared types for episodes, market context, decisions, skills, and configuration
- **Goals** — Trading goal definitions and achievement tracking
- **Notifier** — Notification system for alerts and events
- **Role** — Agent role definitions and hierarchy
- **Skill Aggregator** — Aggregates skill outputs into unified signals
- **Calendar** — Economic calendar and event tracking

## Key Modules

| Module | Purpose |
|--------|---------|
| `disciplined_core.rs` | Hard trading rules + memory-adjusted gates |
| `skills.rs` | `AgentSkill` trait + implementations |
| `memory.rs` | redb-based hot state store |
| `vector_memory.rs` | Semantic similarity + LanceDB integration |
| `paper_engine.rs` | Paper execution + slippage + journaling |
| `broker.rs` | `BrokerAdapter` trait + `BrokerRegistry` |
| `backtest.rs` | CSV backtest engine |
| `llm.rs` | Ollama/OpenAI/Claude async executor |
| `kronos_client.rs` | Forecast sidecar client |
| `patterns.rs` | 15 candlestick pattern detectors |
| `episode.rs` | Trading episode types |
| `agent.rs` | Agent trait definitions |
| `goals.rs` | Trading goal definitions |
| `notifier.rs` | Notification system |
| `role.rs` | Agent role definitions |
| `skill_aggregator.rs` | Skill output aggregation |
| `calendar.rs` | Economic calendar |
| `config.rs` | Configuration + environment |

## Usage

```rust
use tredo_core::DisciplinedCore;
use tredo_core::skills::AgentSkill;
use tredo_core::paper_engine::BrokerAdapter;
```

This crate is a dependency of `tredo-autonomous`, `tredo-orchestrator`, `tredo-tui`, `tredo-server`, `tredo-runtime`, `tredo-broker-alpaca`, and `tredo-broker-zerodha`.
