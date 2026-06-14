use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tredo_core::TradeDirection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CotEntry {
    pub id: u64,
    pub chain_id: u64,
    pub parent_id: Option<u64>,
    pub agent: String,
    pub input: String,
    pub action: String,
    pub reason: String,
    pub confidence: f64,
    pub timestamp: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeSignal {
    pub symbol: String,
    pub direction: TradeDirection,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub position_size: f64,
    pub confidence_score: f64,
    pub confluence_score: f64,
    pub risk_reward_ratio: f64,
    pub reasoning: String,
    pub timestamp: DateTime<Utc>,
    pub session_valid: bool,
    pub risk_check_passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketRegime {
    TrendingBull,
    TrendingBear,
    Ranging,
    Volatile,
    LowLiquidity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioState {
    pub cash_balance: f64,
    pub total_equity: f64,
    pub daily_pnl: f64,
    pub daily_pnl_pct: f64,
    pub open_positions: Vec<OpenPosition>,
    pub total_trades_today: u32,
    pub winning_trades_today: u32,
    pub losing_trades_today: u32,
    pub consecutive_losses: u32,
    pub max_drawdown_today: f64,
    pub last_trade_time: Option<DateTime<Utc>>,
    pub trading_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenPosition {
    pub symbol: String,
    pub direction: TradeDirection,
    pub entry_price: f64,
    pub current_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub quantity: f64,
    pub unrealized_pnl: f64,
    pub unrealized_pnl_pct: f64,
    pub entry_time: DateTime<Utc>,
    pub risk_amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAnalysis {
    pub max_position_size: f64,
    pub risk_per_trade_pct: f64,
    pub risk_reward_ratio: f64,
    pub portfolio_heat: f64,
    pub daily_drawdown_pct: f64,
    pub var_95: f64,
    pub recommendation: RiskRecommendation,
    pub psychology_warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskRecommendation {
    Proceed,
    Caution,
    ReduceSize,
    Halt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub market_open: bool,
    pub session_name: String,
    /// Minutes until close (or open). Stored as i64 to avoid chrono Duration serde issues
    /// in test builds / episode_store contexts (chrono "serde" feature enables DateTime but
    /// Duration requires explicit handling in some derives).
    pub time_to_close: Option<i64>,
    pub time_to_open: Option<i64>,
    pub is_pre_open: bool,
    pub is_post_close: bool,
    pub minutes_since_open: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    pub pattern_key: String,
    pub match_score: f64,
    pub historical_outcome: String,
    pub avg_return: f64,
    pub win_rate: f64,
    pub total_occurrences: usize,
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub phase: String,
    pub passed: bool,
    pub details: Vec<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct PipelineSummary {
    pub executed: bool,
    pub phase_results: Vec<PipelineResult>,
    pub total_duration_ms: u64,
    pub final_signal: Option<TradeSignal>,
    pub reason: String,
}
