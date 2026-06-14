# tredo-autonomous

The intelligence layer of the tredo trading system — agent hierarchy, debate, skills, and temporal pipeline.

## What it provides

- **Agent Hierarchy** — Two-tier architecture with main agents (LLM-capable) and deterministic sub-agents
- **Multi-Agent Debate** — Proposer/Critic/Risk/Historian roles with aggregator for trade signal generation
- **Skills Implementation** — Concrete `AgentSkill` implementations: SentimentAnalyzer, VolatilityCalculator, RegimeDetector, CorrelationChecker, OnChainData
- **Market Intelligence** — Market scanning, pivot/confluence analysis, pattern detection, Kronos forecast integration
- **Reflection & Meta-Control** — Post-trade regret scoring, lesson extraction, automatic rule adaptation
- **Episodic Memory** — SQLite-backed trade journal with regret tracking and rule change history
- **Self-Evolution** — Closed loop: debate → paper execution → reflection → meta rule adaptation
- **Orchestrator Pipeline** — 6-phase pipeline for full cycle execution
- **State Management** — Shared state with OHLCV history, portfolio, rules, COT tree

## Key Modules

| Module | Purpose |
|--------|---------|
| `debate.rs` | 4-role debate engine with aggregator |
| `market_intelligence.rs` | MI agent with skills + trained memory |
| `strategy_decision.rs` | Debate-driven signal generation |
| `reflector.rs` | Post-trade deep reflection |
| `meta_control.rs` | Rule adaptation from regret analysis |
| `episode_store.rs` | SQLite persistent trade journal |
| `risk_calculator.rs` | Position sizing and risk gates |
| `self_evolution.rs` | Self-evolution validation harness |

## Usage

```rust
use tredo_autonomous::debate::DebateCoordinator;
use tredo_autonomous::state::SharedState;
```

Depends on `tredo-core`.
