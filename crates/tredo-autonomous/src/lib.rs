pub mod backtester;
pub mod confluence_scorer;
pub mod drawdown_monitor;
pub mod episode_store;
pub mod execution_coordinator;
pub mod helpers;
pub mod market_intelligence;
pub mod meta_control;
pub mod orchestrator_phases;
pub mod orchestrator_pipeline;
pub mod orchestrator_struct;
pub mod outcome_logger;
pub mod outcome_processor;
pub mod overtrading_preventer;
pub mod pattern_retriever;
pub mod pivot_calculator;
pub mod portfolio_manager;
pub mod red_folder_checker;
pub mod reflector;
pub mod risk_calculator;
pub mod risk_psychology;
pub mod scanner;
pub mod session_timer;
pub mod state;
pub mod strategy_decision;
pub mod tredo;
pub mod types;

// === NEW SKILLS/TOOLS (research upgrades: sentiment, vol, regime for better MI/risk/strategies) ===
pub mod correlation_checker;
pub mod debate;
pub mod on_chain_data; // New on-chain tool for crypto skills (free API stub ready)
pub mod regime_detector;
pub mod sentiment_analyzer;
pub mod volatility_calculator; // Full debate pipeline upgrade (aggregator + 4 agents powered by new skills)

pub use backtester::{AutonomousBacktestResult, AutonomousBacktester};
pub use orchestrator_struct::AutonomousOrchestrator;
pub use state::SharedState;
pub use tredo::{Executer, Guardian, Identifier, Tredo, Verifier};
pub use types::*;
