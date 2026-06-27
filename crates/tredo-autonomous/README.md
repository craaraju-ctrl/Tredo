# tredo-autonomous

The intelligence layer of the tredo trading system — agent hierarchy, debate, skills, and temporal pipeline.

## What it provides

- **Agent Hierarchy** — Two-tier architecture with main agents (LLM-capable) and deterministic sub-agents across 4 groups (Identifier, Verifier, Executer, Guardian)
- **5-Layer Adversarial Pipeline** — `HardRulesGate` (Layer 1) → `RegimeClassifier` (Layer 2) → `DebateLayer` (Layer 3: BullTeam/BearTeam/Synthesizer) → `Judge` (Layer 4) → `Execution` (Layer 5). No LLM dependency — all intelligence is evidence-based and regime-adaptive.
- **HardRulesGate** (`hard_rules_gate.rs`) — Layer 1: Priority-based hard rule enforcement (Critical > High > Medium > Low). Critical/High always block. Medium blocks only if no Higher override. Low = warnings only. Single top-level gate before any advisory layers.
- **Debate Layer** (`debate_layer.rs`) — Layer 3: Multi-round adversarial debate system with BullTeam (12 bullish factors), BearTeam (11 bearish factors), Synthesizer, and Judge. The Judge evaluates debate quality ONLY — does NOT re-run risk/regime/confluence checks (those are handled by HardRulesGate).
- **Autonomous Orchestrator** — `AutonomousOrchestrator` that wraps the full agent pipeline with state management and COT tracking
- **Skills Implementation** — Concrete `AgentSkill` implementations: SentimentAnalyzer, VolatilityCalculator, RegimeDetector, CorrelationChecker, OnChainData, NewsAnalyser, MarketMetricsMeter, **SupportResistance**, **VolumeProfile**, **OrderFlow**, **FundingRate**, **Liquidity**
- **Market Intelligence** — Market scanning, pivot/confluence analysis, pattern detection (27 patterns), Kronos forecast, news analysis, market metrics (26 indicators: Bollinger, ATR, Stochastics, RSI, MACD, ADX, CCI, Williams %R, VWAP, OBV, Parabolic SAR, MFI, CMF, Keltner, Donchian, TEMA, HMA, Elder Ray, Aroon, TRIX, ROC, Momentum)
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
| `hard_rules_gate.rs` | **NEW Layer 1**: Priority-based hard rule gate (Critical/High/Medium/Low) — single top-level enforcement |
| `debate_layer.rs` | **NEW Layer 3**: Multi-round adversarial debate (BullTeam/BearTeam/Synthesizer/Judge) — no LLM dependency |
| `debate.rs` | 4-role debate engine with aggregator |
| `debate_orchestrator.rs` | `DebateOrchestrator` — structured debate rounds + state machine |
| `market_intelligence.rs` | MI agent with skills + trained memory |
| `news_analyser.rs` | Multi-source news sentiment analysis (`AgentSkill`) |
| `sentiment_analyzer.rs` | News sentiment scoring (`AgentSkill`) |
| `volatility_calculator.rs` | ATR + vol expansion detection (`AgentSkill`) |
| `correlation_checker.rs` | Cross-symbol correlation proxy (`AgentSkill`) |
| `on_chain_data.rs` | On-chain signals + local volume proxy (`AgentSkill`) |
| `news_analyser.rs` | Multi-source news sentiment (`AgentSkill`) |
| `market_metrics_meter.rs` | Rich indicator bundle (26 indicators) (`AgentSkill`) |
| `support_resistance.rs` | **NEW**: Swing high/low S/R zone detection (`AgentSkill`) |
| `volume_profile.rs` | **NEW**: POC, VAH, VAL from volume distribution (`AgentSkill`) |
| `order_flow.rs` | **NEW**: Buy/sell pressure imbalance (`AgentSkill`) |
| `funding_rate.rs` | **NEW**: Crypto perp funding counter-sentiment (`AgentSkill`) |
| `liquidity.rs` | **NEW**: Market quality, spread, slippage risk (`AgentSkill`) |
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
