pub mod backtester;
pub mod behavioral_psychology;
pub mod circuit_breaker;
pub mod confluence_scorer;
pub mod deterministic_strategies;
pub mod drawdown_monitor;
pub mod episode_store;
pub mod execution_coordinator;
pub mod helpers;
pub mod live_order_manager;
pub mod market_intelligence;
pub mod meta_control;
pub mod orchestrator_phases;
pub mod orchestrator_pipeline;
pub mod orchestrator_struct;
pub mod outcome_logger;
pub mod outcome_processor;
pub mod overtrading_preventer;
pub mod pattern_retriever;
pub mod pipeline_runner;
pub mod pivot_calculator;
pub mod portfolio_manager;
pub mod reconciliation_engine;
pub mod red_folder_checker;
pub mod reflector;
pub mod risk_calculator;
pub mod risk_guardian;
pub mod risk_psychology;
pub mod scanner;
pub mod self_evolution;
pub mod session_timer;
pub mod state;
pub mod strategy_decision;
pub mod tredo;
pub mod types;
pub mod walk_forward_runner; // Prevents parameter overfitting via train/test rolling windows before paper trading
pub mod weight_tuner; // AttributionEngine + symmetric reward/penalty weight evolution (Layer 4) // Risk parameters that can be evolved / rolled back by MetaControl

// === NEW SKILLS/TOOLS (research upgrades: sentiment, vol, regime for better MI/risk/strategies) ===
pub mod correlation_checker;
pub mod debate;
pub mod debate_layer; // High-level adversarial debate: Bull/Bear teams, Synthesizer, Judge
pub mod funding_rate; // Crypto perpetual funding rate proxy (counter-sentiment indicator)
pub mod hard_rules_gate; // Layer 1: Priority-based hard rules gate (top of pipeline)
pub mod liquidity; // Market liquidity, spread, depth, slippage risk analyzer
pub mod market_metrics_meter; // New: Market Metrics Meter tool - computes rich indicators (RSI/MACD/ATR/BB/Stoch/Vol/Regime/Fib) as pluggable AgentSkill + direct meter for autonomous levels
pub mod news_analyser; // New: integrated News Analyser (uses multi-API NewsFetcher + scores) as AgentSkill + tool, connected to memory/WS/pipeline/aggregator
pub mod on_chain_data; // New on-chain tool for crypto skills (free API stub ready)
pub mod options_surface; // Options chain surface analysis (PCR, skew, max pain)
pub mod order_flow; // Buy/sell pressure imbalance from bar close position + volume
pub mod regime_classifier; // Cognitive Core (Layer 2) — regime understanding belongs here, not in pure data ingestion (Layer 1)
pub mod regime_detector; // kept for backward compat during migration
pub mod sentiment_analyzer;
pub mod skills;
pub mod super_intelligence;
pub mod support_resistance; // Swing high/low S/R level detection with clustering
pub mod tri_level_validator; // Parallel rules + LLM + Kronos validation with outcome attribution
pub mod volatility_calculator;
pub mod volume_profile; // Point of Control (POC), Value Area High/Low (VAH/VAL) // Vol measurement + regime detection

pub use backtester::{AutonomousBacktestResult, AutonomousBacktester};
pub use orchestrator_struct::AutonomousOrchestrator;
pub use pipeline_runner::{
    run_batch, run_single, run_whitelist_loop, BatchPipelineReport, PipelineRunOutcome,
    PipelineRunReport, SymbolCooldownTracker, WhitelistConfig,
};
pub use state::SharedState;
pub use tredo::{Executer, Guardian, Identifier, Tredo, Verifier};
pub use types::*;
