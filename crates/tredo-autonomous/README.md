# tredo-autonomous

The intelligence layer of the tredo trading system — agent hierarchy, debate, skills, and temporal pipeline.

## What it provides

- **Agent Hierarchy** — Two-tier architecture with main agents (LLM-capable) and deterministic sub-agents across 4 groups (Identifier, Verifier, Executer, Guardian)
- **Multi-Agent Debate** — Proposer/Critic/Risk/Historian roles with aggregator for trade signal generation
- **Debate Orchestrator** — `DebateOrchestrator` that manages structured debate rounds, state transitions, and turn scheduling
- **Autonomous Orchestrator** — `AutonomousOrchestrator` that wraps the full agent pipeline with state management and COT tracking
- **Skills Implementation** — Concrete `AgentSkill` implementations: SentimentAnalyzer, VolatilityCalculator, RegimeDetector, CorrelationChecker, OnChainData, NewsAnalyser, MarketMetricsMeter
- **Market Intelligence** — Market scanning, pivot/confluence analysis, pattern detection, Kronos forecast, news analysis, market metrics (Bollinger, ATR, Stochastics, RSI, volume profile)
- **Per-Sub-Agent COT** — All sub-agents push COT entries during pipeline runs with action, confidence, and reasoning
- **Reflection & Meta-Control** — Post-trade regret scoring, lesson extraction, automatic rule adaptation
- **Outcome Processor** — `OutcomeProcessor` that handles trade outcomes, regret scoring, and automatic deep reflection triggering
- **Episodic Memory** — SQLite-backed trade journal with regret tracking and rule change history
- **Self-Evolution** — Closed loop: debate → paper execution → reflection → meta rule adaptation
- **Regime Classification** — `RegimeClassifier` for market regime detection (trending, ranging, volatile, etc.)
- **Risk Guardian** — `RiskGuardian` for advanced portfolio-level risk monitoring
- **Walk-Forward Runner** — `WalkForwardRunner` for out-of-sample validation of strategies
- **Orchestrator Pipeline** — 6-phase pipeline for full cycle execution with chain_id tracking
- **State Management** — Shared state with OHLCV history, portfolio, rules, COT tree, skill votes, aggregated signal

## Key Modules

| Module | Purpose |
|--------|---------|
| `tredo.rs` | Tredo orchestrator with Identifier/Verifier/Executer/Guardian groups |
| `orchestrator_pipeline.rs` | 6-phase pipeline driving full cycle execution |
| `orchestrator.rs` | `AutonomousOrchestrator` — full pipeline wrapper with state + COT |
| `orchestrator_struct.rs` | Struct definitions for the orchestrator |
| `orchestrator_phases.rs` | Phase implementations and transitions |
| `debate.rs` | 4-role debate engine with aggregator |
| `debate_orchestrator.rs` | `DebateOrchestrator` — structured debate rounds + state machine |
| `market_intelligence.rs` | MI agent with skills + trained memory |
| `news_analyser.rs` | Multi-source news sentiment analysis (`AgentSkill`) |
| `market_metrics_meter.rs` | Rich market metrics snapshot (`AgentSkill`) |
| `strategy_decision.rs` | Debate-driven signal generation |
| `reflector.rs` | Post-trade deep reflection |
| `meta_control.rs` | Rule adaptation from regret analysis |
| `episode_store.rs` | SQLite persistent trade journal |
| `risk_calculator.rs` | Position sizing and risk gates |
| `risk_guardian.rs` | Portfolio-level risk monitoring |
| `self_evolution.rs` | Self-evolution validation harness |
| `skills.rs` | ConfluenceScorer + SkillResult aggregation |
| `execution_coordinator_fsm.rs` | FSM-based execution state machine |
| `regime_detector.rs` | Market regime classification |
| `regime_classifier.rs` | Advanced regime classification |
| `weight_tuner.rs` | MetaControl skill weight optimization |
| `outcome_processor.rs` | Trade outcome handling + regret + auto-reflection |
| `walk_forward_runner.rs` | Out-of-sample strategy validation |
| `state.rs` | `SharedState` with portfolio, rules, COT, skill votes |

## Usage

```rust
use tredo_autonomous::debate::DebateCoordinator;
use tredo_autonomous::state::SharedState;
use tredo_autonomous::orchestrator_struct::AutonomousOrchestrator;

// Get the agent hierarchy tree JSON (for TUI display)
let tree = AutonomousOrchestrator::tree_json();
```

Depends on `tredo-core`.
