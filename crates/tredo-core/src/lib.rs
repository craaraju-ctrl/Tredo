pub mod advanced_patterns;
pub mod agent;
pub mod backtest;
pub mod binance;
pub mod broker;
pub mod calendar;
pub mod config;
pub mod disciplined_core;
pub mod episode;
pub mod goals;
pub mod kronos_client;
pub mod live_calendar;
pub mod llm;
pub mod memory;
pub mod messages;
pub mod news;
pub mod notifier;
pub mod options;
pub mod paper_engine;
pub mod patterns;
pub mod portfolio_analytics;
pub mod role;
pub mod service_manager;
pub mod skill_aggregator; // Weighted ensemble aggregation for structured SkillResult outputs
pub mod skills; // New AgentSkill trait for building skills/tools (pluggable agent capabilities)
pub mod graph_rag; // Knowledge Graph for relationship-based recall (symbol→regime→outcome)
pub mod vector_memory; // Vector Memory for similarity search across trading episodes (LanceDB ANN + JSON fallback)

pub use advanced_patterns::{
    detect_advanced_patterns, detect_channel, detect_double_bottom, detect_double_top,
    detect_falling_wedge, detect_flag, detect_head_and_shoulders, detect_pennant,
    detect_rising_wedge, format_advanced_patterns, AdvancedPattern, AdvancedPatternType,
    ChannelPattern, DoubleTopBottomPattern, FlagPennantPattern, HeadShouldersPattern, WedgePattern,
};
pub use agent::{Agent, AgentInput, AgentOutput, AgentTier, SkillDirection};
pub use agentmemory::AgentMemoryClient;
pub use backtest::{BacktestResult, Backtester, TradeDirection, TradeSetup};
pub use binance::{
    fetch_klines, fetch_price as fetch_binance_price, fetch_ticker_24hr, fetch_ticker_24hr_raw,
    fetch_tickers_24hr_batch, is_crypto_symbol, normalize_base_symbol, pair_candidates,
    ticker_to_api_json, to_binance_pair, Ticker24h,
};
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
pub use live_calendar::{fetch_economic_calendar_live, CalendarSource};
pub use llm::{LlmExecutor, LlmTradeDecision};
pub use memory::MemoryStore;
pub use messages::{AgentMessage, LLMRequest, LLMResponse};
pub use news::{NewsContext, NewsFetcher, NewsItem};
pub use options::{
    analyze_options_chain, bear_put_spread, black_scholes_greeks, black_scholes_price,
    bull_call_spread, covered_call, futures_fair_value, implied_volatility, iron_condor,
    long_straddle, long_strangle, protective_put, ExerciseStyle, FuturesContract, OptionContract,
    OptionSide, OptionsChain, OptionsSignal, OptionsStrategy,
};
pub use paper_engine::*;
pub use patterns::{
    detect_patterns, detect_patterns_multi_tf, format_mtf_confirmation, format_patterns,
    CandlestickPattern, ConfirmationLevel, MultiTfPatternConfirmation,
};
pub use portfolio_analytics::{
    efficient_frontier_points, kelly_criterion_fraction, mean_variance_optimize,
    optimal_kelly_portfolio, KellyAllocation, PortfolioVar,
};
pub use role::AgentRole;
pub use service_manager::{ConnectionStatus, ServiceManager, ServiceStatus};
pub use skill_aggregator::{AggregatedSignal, SkillAggregator};
pub use graph_rag::{ClosedEpisodeLite, GraphNode, GraphRecallResult, KnowledgeGraph};
pub use vector_memory::{SimilarResult, VectorEntry, VectorMemory};
pub mod agentmemory;
