# tredo-runtime

The **unified runtime layer** for the tredo autonomous agentic trading system.

This crate transforms tredo from a batch pipeline into a truly event-driven, multi-mode agentic system. It provides the `RuntimeEngine` that wires together all subsystems — orchestrator, event bus, risk manager, world model, policy cache, active learner, and broker plugins — into a single cohesive runtime.

## What it provides

- **RuntimeEngine** — Main orchestrator that wires all subsystems together. Creates the event bus, risk manager, introspector, goal manager, world model, portfolio reasoner, active learner, streaming reasoner, and policy cache. Supports paper, live, backtest, validate, and research modes.
- **Event Bus** — `tokio::broadcast`-based event bus for agent-to-agent communication. `AgentEvent` enum covers market updates, trade signals, risk decisions, COT entries, and system events.
- **Multi-Mode Trading** — `TradingMode` enum: `Paper` (default), `Live` (gated by `--confirm-live`), `Backtest`, `Validate`, `Research`.
- **World Model** — `WorldModelEngine` maintains persistent beliefs about symbols (`SymbolBelief`), cross-symbol correlations (`CrossSymbolBelief`), macro state (`MacroBeliefs`), and active hypotheses (`Hypothesis`). Tracks trend, volatility regime, and smart-money activity.
- **Policy Cache** — Learned (features → action → outcome) lookup table. Records bucketed market features (regime, confluence, RSI, trend, volatility, time-of-day) and uses them to short-circuit expensive Ollama debate when the system has high confidence in a cached decision. Seeded from historical trades, updated after every close.
- **Active Learner** — Exploration budget for uncertain setups. Computes symbol uncertainty, decides when to probe with small positions, and updates uncertainty maps based on outcomes.
- **Introspector** — Self-awareness layer that reports current agent mode (`Explore` / `Exploit` / `Recover`), confidence, and system health.
- **Goal Manager** — Manages trading goals, target achievement, and progress tracking.
- **Portfolio Reasoner** — Reasons about portfolio composition, diversification, and correlation exposure.
- **Streaming Reasoner** — Real-time reasoning over streaming market data for sub-millisecond insights.
- **Risk Manager** — Centralized risk management with per-trade limits, portfolio-level circuit breakers, and safety gates (`RiskRejection`, `RiskDecision`).
- **Resilient Pipeline** — Fault-tolerant pipeline wrapper that handles agent failures, timeouts, and degradation gracefully.
- **Broker Plugin System** — `BrokerPluginManager` discovers and configures broker adapters (`PaperBroker`, `AlpacaBroker`, `ZerodhaKiteBroker`). Supports interactive configuration via CLI.
- **Paper Broker** — `PaperBroker` implementation for virtual trading with realistic slippage and fill modeling.
- **Live Broker** — `LiveBroker` wrapper that dispatches to real broker adapters via the `BrokerRegistry`.
- **Data Feed** — Unified data feed abstraction for market data ingestion.
- **Backtest Feed** — CSV-driven backtest data feed (`timestamp,open,high,low,close,volume`).
- **API Clients** — Shared HTTP client pool with keep-alive, timeout, and retry logic for external APIs.
- **Strategy** — Strategy configuration and parameter management.

## Key Modules

| Module | Purpose |
|--------|---------|
| `engine.rs` | `RuntimeEngine` — main orchestrator, multi-mode run loop, `RunSummary` |
| `event_bus.rs` | `EventBus` + `AgentEvent` — broadcast-based agent communication |
| `mode.rs` | `TradingMode` + `ModeConfig` — paper / live / backtest / validate / research |
| `world_model.rs` | `WorldModelEngine` + `SymbolBelief` + `Hypothesis` — persistent market beliefs |
| `policy_cache.rs` | `PolicyCache` + `MarketFeatures` — learned trading memory |
| `active_learner.rs` | `ActiveLearner` — exploration budget + uncertainty management |
| `introspector.rs` | `Introspector` + `AgentMode` — self-awareness + health |
| `goal_manager.rs` | `GoalManager` — goal tracking + achievement |
| `portfolio_reasoner.rs` | `PortfolioReasoner` — composition + diversification reasoning |
| `streaming_reasoner.rs` | `StreamingReasoner` — real-time sub-millisecond insights |
| `risk_manager.rs` | `RiskManager` + `RiskRejection` + `RiskDecision` — safety gates |
| `resilient_pipeline.rs` | Fault-tolerant pipeline wrapper |
| `broker/` | `BrokerPluginManager` + `BrokerConfig` + plugin registry + sandbox |
| `paper_broker.rs` | `PaperBroker` — virtual trading with slippage |
| `live_broker.rs` | `LiveBroker` — dispatches to real broker adapters |
| `data_feed.rs` | Unified market data feed abstraction |
| `backtest_feed.rs` | CSV backtest feed |
| `api_clients.rs` | Shared HTTP client pool |
| `strategy.rs` | Strategy configuration |

## Usage

```bash
# Build the runtime
cargo build -p tredo-runtime --release

# Run in paper mode (default)
./target/release/tredo --mode paper

# Run in validate mode with regret induction
./target/release/tredo --mode validate --cycles 100 --induce-regret

# Backtest mode
./target/release/tredo --mode backtest --data ./btc_2024.csv --capital 100000

# Live mode (requires --confirm-live)
./target/release/tredo --mode live --confirm-live

# Broker configuration
./target/release/tredo configure alpaca
./target/release/tredo configure zerodha

# List available brokers
./target/release/tredo list
```

```rust
use tredo_runtime::engine::RuntimeEngine;
use tredo_runtime::mode::{ModeConfig, TradingMode};
use tredo_autonomous::AutonomousOrchestrator;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mode = ModeConfig::new(TradingMode::Paper);
    let orchestrator = AutonomousOrchestrator::new().await?;
    let engine = RuntimeEngine::new(mode, orchestrator, vec!["BTC".into()], None).await?;
    let summary = engine.run().await?;
    println!("Run complete: {} trades, P&L {:.2}", summary.trades_executed, summary.total_pnl);
    Ok(())
}
```

## CLI

The runtime crate provides the unified `tredo` binary (using `clap`):

| Flag | Description |
|------|-------------|
| `--mode <MODE>` | `paper` (default), `live`, `backtest`, `validate`, `research` |
| `--confirm-live` | REQUIRED for live mode |
| `--data <PATH>` | CSV path for backtest mode |
| `--capital <N>` | Starting capital for backtest (default 100000) |
| `--cycles <N>` | Number of cycles for validate mode (default 50) |
| `--induce-regret` | Force regret-inducing conditions in validate mode |
| `--max-daily-loss <N>` | Max daily loss limit (default 1000) |

Depends on `tredo-core`, `tredo-autonomous`, `tredo-broker-zerodha`, `tredo-broker-alpaca`.
