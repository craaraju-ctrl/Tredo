pub mod agent;
pub mod backtest;
pub mod broker;
pub mod calendar;
pub mod config;
pub mod disciplined_core;
pub mod episode;
pub mod goals;
pub mod kronos_client;
pub mod llm;
pub mod memory;
pub mod messages;
pub mod news;
pub mod notifier;
pub mod paper_engine;
pub mod patterns;
pub mod role;
pub mod skill_aggregator; // Weighted ensemble aggregation for structured SkillResult outputs
pub mod skills; // New AgentSkill trait for building skills/tools (pluggable agent capabilities)
pub mod vector_memory; // Vector Memory for similarity search across trading episodes (LanceDB ANN + JSON fallback)

pub use agent::{Agent, AgentInput, AgentOutput, AgentTier, SkillDirection};
pub use agentmemory::AgentMemoryClient;
pub use backtest::{BacktestResult, Backtester, TradeDirection, TradeSetup};
pub use calendar::{generate_economic_calendar, CalendarEvent, EventImpact};
pub use config::Config;
pub use disciplined_core::{
    apply_trained_memory_to_rules, calculate_confluence_score, calculate_pivot_points,
    check_risk_limits, get_discipline_summary, is_in_trading_session, validate_trade_setup,
    DisciplineCheck, DisciplineRules, MarketContext, PivotLevels, PivotMethod, SkillVote,
    TrendDirection,
};
pub use episode::{
    MarketStateSnapshot, PostTradeReflection, ReasoningStep, TradeOutcome, TradingEpisode,
};
pub use goals::{TradingGoals, TradingMode};
pub use kronos_client::{
    KronosClient, KronosForecastRequest, KronosForecastResponse, KronosForecastTool, OhlcvBar,
};
pub use llm::{LlmExecutor, LlmTradeDecision};
pub use memory::MemoryStore;
pub use messages::{AgentMessage, LLMRequest, LLMResponse};
pub use news::{NewsContext, NewsFetcher, NewsItem};
pub use paper_engine::*;
pub use patterns::{
    detect_patterns, detect_patterns_multi_tf, format_mtf_confirmation, format_patterns,
    CandlestickPattern, ConfirmationLevel, MultiTfPatternConfirmation,
};
pub use role::AgentRole;
pub use skill_aggregator::{AggregatedSignal, SkillAggregator};
pub use vector_memory::{SimilarResult, VectorEntry, VectorMemory};
pub mod agentmemory;
