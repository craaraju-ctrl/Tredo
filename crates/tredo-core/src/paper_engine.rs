//! # PaperEngine — Virtual Portfolio & Order Matching Engine
//!
//! Production-grade paper trading engine that mirrors a real brokerage account.
//! The **exact same code path** is used for paper and live trading.
//! The only difference is which `BrokerAdapter` implementation handles execution.
//!
//! ## Architecture
//! ```text
//! StrategyEngine (JS/Rust) → TradeSignal
//!     → BrokerAdapter::place_order(order)
//!         → PaperBroker (PaperEngine)  [PAPER MODE]
//!         → LiveBroker (TODO)         [LIVE MODE]
//! ```
//!
//! PaperEngine features:
//! - Real-time P&L tracking (realized + unrealized)
//! - Stop-loss / take-profit monitoring
//! - Risk checks (drawdown, heat, concentration)
//! - Trade journal with full history
//! - Supports MULTI and SHORT positions
//! - FIFO position closure
//! - Configurable initial balance

use crate::TradeDirection;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Position Status ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionStatus {
    Open,
    Closed,
    StoppedOut,
    TakeProfit,
    Cancelled,
}

impl std::fmt::Display for PositionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PositionStatus::Open => write!(f, "OPEN"),
            PositionStatus::Closed => write!(f, "CLOSED"),
            PositionStatus::StoppedOut => write!(f, "STOPPED_OUT"),
            PositionStatus::TakeProfit => write!(f, "TAKE_PROFIT"),
            PositionStatus::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

// Uses `crate::TradeDirection` (from backtest module) to avoid type collision
// within the tredo-core crate. Paper/live share the exact same direction type.

// ── Order Types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub direction: TradeDirection,
    pub order_type: OrderType,
    pub qty: i32,
    pub price: Option<f64>, // For limit orders
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub strategy: Option<String>,
    pub client_order_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    StopLoss,
    StopLossLimit,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderType::Market => write!(f, "MARKET"),
            OrderType::Limit => write!(f, "LIMIT"),
            OrderType::StopLoss => write!(f, "STOP_LOSS"),
            OrderType::StopLossLimit => write!(f, "STOP_LOSS_LIMIT"),
        }
    }
}

// ── Order Status ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    Pending,
    Accepted,
    Filled,
    PartiallyFilled { filled_qty: i32 },
    Rejected { reason: String },
    Cancelled,
    Expired,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Pending => write!(f, "PENDING"),
            OrderStatus::Accepted => write!(f, "ACCEPTED"),
            OrderStatus::Filled => write!(f, "FILLED"),
            OrderStatus::PartiallyFilled { filled_qty } => {
                write!(f, "PARTIALLY_FILLED (qty={})", filled_qty)
            }
            OrderStatus::Rejected { reason } => write!(f, "REJECTED ({})", reason),
            OrderStatus::Cancelled => write!(f, "CANCELLED"),
            OrderStatus::Expired => write!(f, "EXPIRED"),
        }
    }
}

// ── Position (Open) ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub symbol: String,
    pub direction: TradeDirection,
    pub qty: i32,
    pub entry_price: f64,
    pub current_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub unrealized_pnl: f64,
    pub unrealized_pnl_pct: f64,
    pub status: PositionStatus,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub strategy: Option<String>,
    pub order_id: String,
}

// ── Closed Trade (Journal) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub id: String,
    pub symbol: String,
    pub direction: TradeDirection,
    pub qty: i32,
    pub entry_price: f64,
    pub exit_price: f64,
    pub realized_pnl: f64,
    pub realized_pnl_pct: f64,
    pub close_reason: CloseReason,
    pub opened_at: DateTime<Utc>,
    pub closed_at: DateTime<Utc>,
    pub duration_secs: i64,
    pub strategy: Option<String>,
    pub order_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    Manual,
    StopLoss,
    TakeProfit,
    Expired,
}

impl std::fmt::Display for CloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloseReason::Manual => write!(f, "MANUAL"),
            CloseReason::StopLoss => write!(f, "STOP_LOSS"),
            CloseReason::TakeProfit => write!(f, "TAKE_PROFIT"),
            CloseReason::Expired => write!(f, "EXPIRED"),
        }
    }
}

// ── Level2 Order Book Types (Realistic Fill Simulation) ─────────────────────

/// Result of walking the order book for a market order fill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillResult {
    /// Average fill price across all consumed levels
    pub avg_fill_price: f64,
    /// Total quantity that was actually filled
    pub filled_qty: f64,
    /// Slippage % relative to the best opposing price (e.g., 0.05 = 0.05%)
    pub slippage_pct: f64,
    /// Number of price levels consumed to fill the order
    pub levels_consumed: usize,
    /// Whether the full order was filled
    pub fully_filled: bool,
}

/// A single price level in the order book.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DepthLevel {
    pub price: f64,
    pub quantity: f64,
}

/// Local order book — maintains sorted bid/ask levels for realistic fill simulation.
///
/// Bids are stored highest-price-first (best bid priority).
/// Asks are stored lowest-price-first (best ask priority).
/// Use `apply_snapshot` to initialize from a full depth snapshot,
/// then `apply_depth_update` for incremental Binance-style updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct LocalOrderBook {
    /// Bids sorted descending by price
    pub bids: Vec<DepthLevel>,
    /// Asks sorted ascending by price
    pub asks: Vec<DepthLevel>,
    /// Last update ID for consistency checking with Binance-style streams
    pub last_update_id: u64,
}


impl LocalOrderBook {
    /// Create a new empty order book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a full snapshot — replaces all levels.
    /// `bids` and `asks` are (price, quantity) pairs.
    /// Levels with quantity ≈ 0.0 are filtered out.
    pub fn apply_snapshot(&mut self, bids: Vec<(f64, f64)>, asks: Vec<(f64, f64)>, update_id: u64) {
        self.bids = bids
            .into_iter()
            .filter(|(_, qty)| *qty > 0.0)
            .map(|(p, q)| DepthLevel { price: p, quantity: q })
            .collect();
        self.asks = asks
            .into_iter()
            .filter(|(_, qty)| *qty > 0.0)
            .map(|(p, q)| DepthLevel { price: p, quantity: q })
            .collect();
        // Sort bids descending (best bid first), asks ascending (best ask first)
        self.bids
            .sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        self.asks
            .sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
        self.last_update_id = update_id;
    }

    /// Apply incremental depth updates (Binance @depth stream style).
    /// Levels with quantity=0.0 are removed. Positive quantities update existing or insert new.
    /// After applying, re-sorts both sides to maintain correct ordering.
    pub fn apply_depth_update(&mut self, bids: Vec<(f64, f64)>, asks: Vec<(f64, f64)>, update_id: u64) {
        Self::apply_level_updates(&mut self.bids, bids, false);
        Self::apply_level_updates(&mut self.asks, asks, true);
        self.last_update_id = update_id;
    }

    fn apply_level_updates(levels: &mut Vec<DepthLevel>, updates: Vec<(f64, f64)>, is_asks: bool) {
        // Build a map of price → quantity for the updates
        let update_map: std::collections::BTreeMap<PriceKey, f64> = updates
            .into_iter()
            .map(|(p, q)| (price_to_key(p), q))
            .collect();

        // Remove levels where update quantity is zero
        levels.retain(|l| !update_map.get(&price_to_key(l.price)).is_some_and(|&q| q <= 0.0));

        // Update existing levels and add new ones
        for (key, qty) in update_map {
            if qty <= 0.0 {
                continue;
            }
            let price = key_to_price(key);
            if let Some(existing) = levels.iter_mut().find(|l| (l.price - price).abs() < 1e-8) {
                existing.quantity = qty;
            } else {
                levels.push(DepthLevel { price, quantity: qty });
            }
        }

        // Re-sort
        if is_asks {
            levels.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            levels.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
        }
    }

    /// Simulate a market BUY: walk the ask side starting from best (lowest) ask.
    /// Returns average fill price, filled quantity, and slippage % from top of book.
    pub fn market_buy(&self, qty: f64) -> FillResult {
        if qty <= 0.0 || self.asks.is_empty() {
            let best = self.asks.first().copied().unwrap_or(DepthLevel { price: 0.0, quantity: 0.0 });
            return FillResult {
                avg_fill_price: best.price,
                filled_qty: 0.0,
                slippage_pct: 0.0,
                levels_consumed: 0,
                fully_filled: false,
            };
        }

        let best_ask = self.asks[0].price;
        let mut remaining = qty;
        let mut total_cost = 0.0;
        let mut levels_used = 0;

        for level in &self.asks {
            if remaining <= 0.0 {
                break;
            }
            let fill = remaining.min(level.quantity);
            total_cost += fill * level.price;
            remaining -= fill;
            levels_used += 1;
        }

        let filled = qty - remaining;
        let avg_price = if filled > 0.0 { total_cost / filled } else { best_ask };
        let slippage = if best_ask > 0.0 {
            ((avg_price - best_ask) / best_ask) * 100.0
        } else {
            0.0
        };

        FillResult {
            avg_fill_price: avg_price,
            filled_qty: filled,
            slippage_pct: slippage.max(0.0),
            levels_consumed: levels_used,
            fully_filled: remaining <= 0.0,
        }
    }

    /// Simulate a market SELL: walk the bid side starting from best (highest) bid.
    pub fn market_sell(&self, qty: f64) -> FillResult {
        if qty <= 0.0 || self.bids.is_empty() {
            let best = self.bids.first().copied().unwrap_or(DepthLevel { price: 0.0, quantity: 0.0 });
            return FillResult {
                avg_fill_price: best.price,
                filled_qty: 0.0,
                slippage_pct: 0.0,
                levels_consumed: 0,
                fully_filled: false,
            };
        }

        let best_bid = self.bids[0].price;
        let mut remaining = qty;
        let mut total_proceeds = 0.0;
        let mut levels_used = 0;

        for level in &self.bids {
            if remaining <= 0.0 {
                break;
            }
            let fill = remaining.min(level.quantity);
            total_proceeds += fill * level.price;
            remaining -= fill;
            levels_used += 1;
        }

        let filled = qty - remaining;
        let avg_price = if filled > 0.0 { total_proceeds / filled } else { best_bid };
        let slippage = if best_bid > 0.0 {
            ((best_bid - avg_price) / best_bid) * 100.0
        } else {
            0.0
        };

        FillResult {
            avg_fill_price: avg_price,
            filled_qty: filled,
            slippage_pct: slippage.max(0.0),
            levels_consumed: levels_used,
            fully_filled: remaining <= 0.0,
        }
    }

    /// Best bid (highest bid price)
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|l| l.price)
    }

    /// Best ask (lowest ask price)
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|l| l.price)
    }

    /// Mid price (average of best bid and best ask)
    pub fn mid_price(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            (Some(p), None) | (None, Some(p)) => Some(p),
            _ => None,
        }
    }

    /// Spread as percentage of mid price
    pub fn spread_pct(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask(), self.mid_price()) {
            (Some(bid), Some(ask), Some(mid)) if mid > 0.0 => Some(((ask - bid) / mid) * 100.0),
            _ => None,
        }
    }

    /// Total bid volume (sum of all bid quantities)
    pub fn total_bid_volume(&self) -> f64 {
        self.bids.iter().map(|l| l.quantity).sum()
    }

    /// Total ask volume
    pub fn total_ask_volume(&self) -> f64 {
        self.asks.iter().map(|l| l.quantity).sum()
    }

    /// Bid/Ask ratio (>1.0 = more bid liquidity, <1.0 = more ask liquidity)
    pub fn bid_ask_ratio(&self) -> f64 {
        let ask_vol = self.total_ask_volume();
        if ask_vol > 0.0 {
            self.total_bid_volume() / ask_vol
        } else if self.total_bid_volume() > 0.0 {
            f64::MAX
        } else {
            1.0
        }
    }

    /// Estimate slippage for a market order of given size.
    /// Returns the estimated slippage % vs top-of-book.
    pub fn estimate_slippage(&self, qty: f64, is_buy: bool) -> f64 {
        let result = if is_buy {
            self.market_buy(qty)
        } else {
            self.market_sell(qty)
        };
        result.slippage_pct
    }
}

/// Price key for BTreeMap — use integer to avoid floating-point comparison issues.
type PriceKey = i64;

fn price_to_key(price: f64) -> PriceKey {
    (price * 100_000_000.0).round() as i64
}

fn key_to_price(key: PriceKey) -> f64 {
    key as f64 / 100_000_000.0
}

// ── Portfolio Summary ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortfolioSummary {
    pub cash: f64,
    pub equity: f64,
    pub margin_used: f64,
    pub free_margin: f64,
    pub daily_pnl: f64,
    pub daily_pnl_pct: f64,
    pub total_trades: u32,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub win_rate: f64,
    pub consecutive_losses: u32,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub open_positions: usize,
    pub total_pnl_all_time: f64,
}

// ── Risk Check Result ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskCheckResult {
    pub passed: bool,
    pub max_position_size_ok: bool,
    pub daily_loss_limit_ok: bool,
    pub drawdown_ok: bool,
    pub concentration_ok: bool,
    pub portfolio_heat_ok: bool,
    pub warnings: Vec<String>,
}

// ── Trading Mode ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingMode {
    Paper,
    Live,
}

impl std::fmt::Display for TradingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradingMode::Paper => write!(f, "PAPER"),
            TradingMode::Live => write!(f, "LIVE"),
        }
    }
}

// ── Engine Configuration ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperEngineConfig {
    pub initial_balance: f64,
    pub max_position_size_pct: f64,  // % of equity per position
    pub max_daily_loss_pct: f64,     // % daily loss limit
    pub max_drawdown_pct: f64,       // % max drawdown
    pub max_concentration_pct: f64,  // % in single symbol
    pub max_portfolio_heat_pct: f64, // % total risk exposure
    pub max_leverage: f64,
    pub slippage_model: SlippageModel,
    pub commission_pct: f64,            // % commission per trade
    /// Enable Level2 realistic fill simulation. When true, place_order
    /// walks the local order book to determine fills instead of fixed slippage.
    pub realistic_paper_enabled: bool,
}

impl Default for PaperEngineConfig {
    fn default() -> Self {
        Self {
            initial_balance: 100_000.0,
            max_position_size_pct: 4.0,    // 1/25 = 4% max per position
            max_daily_loss_pct: 3.0,
            max_drawdown_pct: 10.0,
            max_concentration_pct: 4.0,    // 1/25 = 4% max per symbol
            max_portfolio_heat_pct: 30.0,
            max_leverage: 1.0,
            slippage_model: SlippageModel::Fixed(0.01), // 1 paisa slippage
            commission_pct: 0.03,                       // 0.03% (like Zerodha)
            realistic_paper_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SlippageModel {
    None,
    Fixed(f64),      // Fixed amount per share
    Percentage(f64), // % of trade value
}

/// Apply slippage to a market price based on the configured model.
fn apply_slippage(model: &SlippageModel, price: f64, direction: TradeDirection) -> f64 {
    match model {
        SlippageModel::None => price,
        SlippageModel::Fixed(s) => {
            if direction == TradeDirection::Long {
                price + s
            } else {
                price - s
            }
        }
        SlippageModel::Percentage(p) => {
            let slippage = price * (p / 100.0);
            if direction == TradeDirection::Long {
                price + slippage
            } else {
                price - slippage
            }
        }
    }
}

// ── PaperEngine ──────────────────────────────────────────────────────────────

/// The core paper trading engine. Manages a virtual portfolio with full
/// order lifecycle, P&L tracking, risk checks, and trade journal.
///
/// This is the **same engine** used whether trading paper or live.
/// Live mode swaps out the execution layer but keeps the same
/// portfolio management, risk checks, and P&L calculations.
#[derive(Debug)]
pub struct PaperEngine {
    pub config: PaperEngineConfig,
    pub portfolio: RwLock<Portfolio>,
    /// Local order books keyed by symbol. Populated by depth feed.
    /// When `config.realistic_paper_enabled` is true, `place_order` walks
    /// the book for realistic fill simulation instead of fixed slippage.
    pub order_books: RwLock<HashMap<String, LocalOrderBook>>,
    order_counter: AtomicU64,
    trade_counter: AtomicU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub cash: f64,
    pub equity: f64,
    pub daily_pnl: f64,
    pub total_trades: u32,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub consecutive_losses: u32,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub initial_balance: f64,
    pub positions: Vec<Position>,
    pub trade_history: Vec<ClosedTrade>,
    pub orders: Vec<OrderRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRecord {
    pub order_id: String,
    pub request: OrderRequest,
    pub status: OrderStatus,
    pub filled_price: Option<f64>,
    pub filled_qty: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PaperEngine {
    /// Create a new paper engine with the given configuration.
    pub fn new(config: PaperEngineConfig) -> Self {
        let initial = config.initial_balance;
        Self {
            config,
            portfolio: RwLock::new(Portfolio {
                cash: initial,
                equity: initial,
                daily_pnl: 0.0,
                total_trades: 0,
                winning_trades: 0,
                losing_trades: 0,
                consecutive_losses: 0,
                max_drawdown: initial,
                max_drawdown_pct: 0.0,
                initial_balance: initial,
                positions: Vec::new(),
                trade_history: Vec::new(),
                orders: Vec::new(),
            }),
            order_books: RwLock::new(HashMap::new()),
            order_counter: AtomicU64::new(1),
            trade_counter: AtomicU64::new(1),
        }
    }

    /// Reset the engine to initial state (for testing / start fresh).
    pub async fn reset(&self) {
        let mut p = self.portfolio.write().await;
        let initial = self.config.initial_balance;
        p.cash = initial;
        p.equity = initial;
        p.daily_pnl = 0.0;
        p.total_trades = 0;
        p.winning_trades = 0;
        p.losing_trades = 0;
        p.consecutive_losses = 0;
        p.max_drawdown = initial;
        p.max_drawdown_pct = 0.0;
        p.positions.clear();
        p.trade_history.clear();
        p.orders.clear();
        // Also clear order books
        self.order_books.write().await.clear();
    }

    // ── Order Book Management ────────────────────────────────────────────

    /// Apply a full depth snapshot to the local order book for a symbol.
    /// This initializes or resets the book with the given bid/ask levels.
    pub async fn apply_depth_snapshot(
        &self,
        symbol: &str,
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
        update_id: u64,
    ) {
        let mut books = self.order_books.write().await;
        let book = books.entry(symbol.to_string()).or_default();
        book.apply_snapshot(bids, asks, update_id);
        println!(
            "[PaperEngine] 🏛️  L2 snapshot for {}: {} bids, {} asks (ID: {})",
            symbol,
            book.bids.len(),
            book.asks.len(),
            update_id
        );
    }

    /// Apply incremental depth updates to the local order book.
    /// Called by the depth feed (Binance WS depth@100ms or REST polling).
    pub async fn apply_depth_update(
        &self,
        symbol: &str,
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
        update_id: u64,
    ) {
        let mut books = self.order_books.write().await;
        let book = books.entry(symbol.to_string()).or_default();
        book.apply_depth_update(bids, asks, update_id);
    }

    /// Get a snapshot of the current order book for a symbol.
    pub async fn get_order_book(&self, symbol: &str) -> Option<LocalOrderBook> {
        let books = self.order_books.read().await;
        books.get(symbol).cloned()
    }

    /// Estimate realistic slippage for a market order using the local order book.
    /// Returns None if the book is not populated for this symbol.
    pub async fn estimate_realistic_slippage(&self, symbol: &str, qty: f64, is_buy: bool) -> Option<FillResult> {
        let books = self.order_books.read().await;
        let book = books.get(symbol)?;
        if book.bids.is_empty() || book.asks.is_empty() {
            return None;
        }
        Some(if is_buy {
            book.market_buy(qty)
        } else {
            book.market_sell(qty)
        })
    }

    // ── Order Placement ───────────────────────────────────────────────────

    /// Place a market or limit order. Returns the order ID.
    ///
    /// When `config.realistic_paper_enabled` is true AND the local order book
    /// has data for the symbol, this method walks the book to compute a
    /// realistic fill price (walk-the-book simulation). Otherwise falls back
    /// to the configured `SlippageModel` (Fixed/Percentage/None).
    pub async fn place_order(
        &self,
        request: OrderRequest,
        market_price: f64,
    ) -> Result<String, String> {
        // Validate
        if request.qty <= 0 {
            return Err("Quantity must be positive".to_string());
        }

        // Generate order ID
        let order_id = format!(
            "ORD-{:06}",
            self.order_counter.fetch_add(1, Ordering::SeqCst)
        );
        let now = Utc::now();

        // ── Try realistic LOB fill first (if enabled with populated book) ────
        // This computes both the fill price AND the actual filled quantity from
        // walking the order book. If partial fill occurs, the position quantity
        // reflects what was actually filled in the market.
        let (fill_price, fill_result_opt) = match request.order_type {
            OrderType::Market if self.config.realistic_paper_enabled => {
                let books = self.order_books.read().await;
                let result = books.get(&request.symbol).and_then(|book| {
                    if book.bids.is_empty() || book.asks.is_empty() {
                        None
                    } else {
                        let is_buy = request.direction == TradeDirection::Long;
                        Some(if is_buy {
                            book.market_buy(request.qty as f64)
                        } else {
                            book.market_sell(request.qty as f64)
                        })
                    }
                });
                drop(books);

                match result {
                    Some(r) if r.filled_qty > 0.0 => {
                        println!(
                            "[PaperEngine] 🏛️  L2 fill: {} {} @ {:.4} (slippage: {:.3}%, levels: {}, fully_filled: {})",
                            request.symbol,
                            if request.direction == TradeDirection::Long { "BUY" } else { "SELL" },
                            r.avg_fill_price,
                            r.slippage_pct,
                            r.levels_consumed,
                            r.fully_filled
                        );
                        (r.avg_fill_price, Some(r))
                    }
                    _ => (apply_slippage(&self.config.slippage_model, market_price, request.direction), None),
                }
            }
            OrderType::Limit => (request.price.unwrap_or(market_price), None),
            OrderType::StopLoss | OrderType::StopLossLimit => (market_price, None),
            _ => (apply_slippage(&self.config.slippage_model, market_price, request.direction), None),
        };

        // Use the effective filled quantity from LOB simulation, or the full requested qty
        let effective_qty = fill_result_opt
            .as_ref()
            .map(|r| (r.filled_qty as i32).max(1))
            .unwrap_or(request.qty);
        let effective_cost = fill_price * effective_qty as f64;

        // Check cash balance
        let mut portfolio = self.portfolio.write().await;

        if portfolio.cash < effective_cost {
            return Err(format!(
                "Insufficient cash. Need ₹{:.2}, have ₹{:.2}",
                effective_cost, portfolio.cash
            ));
        }

        // Apply commission
        let commission = effective_cost * (self.config.commission_pct / 100.0);

        // Deduct cash
        portfolio.cash -= effective_cost + commission;

        // Create position with effective qty (may differ from requested if LOB partial fill)
        let position = Position {
            id: format!(
                "POS-{:06}",
                self.trade_counter.fetch_add(1, Ordering::SeqCst)
            ),
            symbol: request.symbol.clone(),
            direction: request.direction,
            qty: effective_qty,
            entry_price: fill_price,
            current_price: fill_price,
            stop_loss: request.stop_loss.unwrap_or(0.0),
            take_profit: request.take_profit.unwrap_or(0.0),
            unrealized_pnl: 0.0,
            unrealized_pnl_pct: 0.0,
            status: PositionStatus::Open,
            opened_at: now,
            closed_at: None,
            strategy: request.strategy.clone(),
            order_id: order_id.clone(),
        };

        // Mark order as filled (use effective_qty for record, show LOB info if available)
        let order_status = if let Some(ref r) = fill_result_opt {
            if r.fully_filled {
                OrderStatus::Filled
            } else {
                OrderStatus::PartiallyFilled {
                    filled_qty: effective_qty,
                }
            }
        } else {
            OrderStatus::Filled
        };
        let order_record = OrderRecord {
            order_id: order_id.clone(),
            request: request.clone(),
            status: order_status,
            filled_price: Some(fill_price),
            filled_qty: effective_qty,
            created_at: now,
            updated_at: now,
        };

        portfolio.orders.push(order_record);
        portfolio.positions.push(position);

        // Recalculate equity
        self.recalculate_equity(&mut portfolio).await;

        Ok(order_id)
    }

    // ── Position Monitoring ───────────────────────────────────────────────

    /// Update all open positions with the latest market price for a symbol.
    /// Returns any positions that were closed (SL/TP hit).
    pub async fn update_price(&self, symbol: &str, market_price: f64) -> Vec<ClosedTrade> {
        let mut closed = Vec::new();
        let mut portfolio = self.portfolio.write().await;

        let mut i = 0;
        while i < portfolio.positions.len() {
            let pos_clone = portfolio.positions[i].clone();
            if pos_clone.symbol != symbol || pos_clone.status != PositionStatus::Open {
                i += 1;
                continue;
            }

            // Update current price and P&L
            portfolio.positions[i].current_price = market_price;
            let (upnl, upnl_pct) = Self::calculate_pnl(
                pos_clone.direction,
                pos_clone.entry_price,
                market_price,
                pos_clone.qty,
            );
            portfolio.positions[i].unrealized_pnl = upnl;
            portfolio.positions[i].unrealized_pnl_pct = upnl_pct;

            // Check stop-loss (using cloned data to avoid borrow conflict)
            if pos_clone.stop_loss > 0.0 {
                let stop_hit = match pos_clone.direction {
                    TradeDirection::Long => market_price <= pos_clone.stop_loss,
                    TradeDirection::Short => market_price >= pos_clone.stop_loss,
                };
                if stop_hit {
                    let closed_trade = self.close_position_internal(
                        &mut portfolio,
                        i,
                        CloseReason::StopLoss,
                        pos_clone.stop_loss,
                    );
                    closed.push(closed_trade);
                    continue;
                }
            }

            // Check take-profit
            if pos_clone.take_profit > 0.0 {
                let tp_hit = match pos_clone.direction {
                    TradeDirection::Long => market_price >= pos_clone.take_profit,
                    TradeDirection::Short => market_price <= pos_clone.take_profit,
                };
                if tp_hit {
                    let closed_trade = self.close_position_internal(
                        &mut portfolio,
                        i,
                        CloseReason::TakeProfit,
                        pos_clone.take_profit,
                    );
                    closed.push(closed_trade);
                    continue;
                }
            }

            i += 1;
        }

        self.recalculate_equity(&mut portfolio).await;
        closed
    }

    /// Manually close a position by its ID.
    pub async fn close_position(
        &self,
        position_id: &str,
        exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        let mut portfolio = self.portfolio.write().await;
        let idx = portfolio
            .positions
            .iter()
            .position(|p| p.id == position_id)
            .ok_or_else(|| format!("Position {} not found", position_id))?;
        Ok(self.close_position_internal(&mut portfolio, idx, CloseReason::Manual, exit_price))
    }

    /// Internal position close (removes from portfolio.positions, adds to trade_history)
    fn close_position_internal(
        &self,
        portfolio: &mut Portfolio,
        idx: usize,
        reason: CloseReason,
        exit_price: f64,
    ) -> ClosedTrade {
        let pos = &portfolio.positions[idx];
        let now = Utc::now();

        let (realized_pnl, realized_pnl_pct) =
            Self::calculate_pnl(pos.direction, pos.entry_price, exit_price, pos.qty);

        let duration = (now - pos.opened_at).num_seconds();

        // Apply commission on exit
        let commission = exit_price * pos.qty as f64 * (self.config.commission_pct / 100.0);
        let net_pnl = realized_pnl - commission;

        let closed_trade = ClosedTrade {
            id: pos.id.clone(),
            symbol: pos.symbol.clone(),
            direction: pos.direction,
            qty: pos.qty,
            entry_price: pos.entry_price,
            exit_price,
            realized_pnl: net_pnl,
            realized_pnl_pct,
            close_reason: reason,
            opened_at: pos.opened_at,
            closed_at: now,
            duration_secs: duration,
            strategy: pos.strategy.clone(),
            order_id: pos.order_id.clone(),
        };

        // Update portfolio stats: credit the sale proceeds minus exit commission.
        // The entry cost was already deducted on buy, so P&L is implicit in the
        // difference between entry_cost (deducted) and exit_proceeds (credited).
        portfolio.cash += exit_price * pos.qty as f64 - commission;
        portfolio.daily_pnl += net_pnl;
        portfolio.total_trades += 1;

        if net_pnl >= 0.0 {
            portfolio.winning_trades += 1;
            portfolio.consecutive_losses = 0;
        } else {
            portfolio.losing_trades += 1;
            portfolio.consecutive_losses += 1;
        }

        // Track drawdown
        if portfolio.equity < portfolio.max_drawdown {
            portfolio.max_drawdown = portfolio.equity;
            let dd = (portfolio.max_drawdown - portfolio.initial_balance)
                / portfolio.initial_balance
                * 100.0;
            if dd < portfolio.max_drawdown_pct {
                portfolio.max_drawdown_pct = dd;
            }
        }

        // Mark position as closed
        portfolio.positions[idx].status = match reason {
            CloseReason::StopLoss => PositionStatus::StoppedOut,
            CloseReason::TakeProfit => PositionStatus::TakeProfit,
            CloseReason::Manual => PositionStatus::Closed,
            CloseReason::Expired => PositionStatus::Cancelled,
        };
        portfolio.positions[idx].closed_at = Some(now);

        // Remove from open positions and add to history
        portfolio.positions.remove(idx);
        portfolio.trade_history.push(closed_trade.clone());

        closed_trade
    }

    // ── P&L Calculations ──────────────────────────────────────────────────

    fn calculate_pnl(direction: TradeDirection, entry: f64, exit: f64, qty: i32) -> (f64, f64) {
        let diff = match direction {
            TradeDirection::Long => exit - entry,
            TradeDirection::Short => entry - exit,
        };
        let pnl = diff * qty as f64;
        let invested = entry * qty as f64;
        let pnl_pct = if invested > 0.0 {
            (pnl / invested) * 100.0
        } else {
            0.0
        };
        (pnl, pnl_pct)
    }

    async fn recalculate_equity(&self, portfolio: &mut Portfolio) {
        // Equity = cash + current market value of all open positions.
        // We use market value (current_price * qty), NOT unrealized P&L,
        // because cash already has the entry cost deducted.
        let total_market_value: f64 = portfolio
            .positions
            .iter()
            .map(|pos| pos.current_price * pos.qty as f64)
            .sum();
        portfolio.equity = portfolio.cash + total_market_value;
    }

    // ── Risk Checks ───────────────────────────────────────────────────────

    pub async fn check_risk(&self, symbol: &str, estimated_cost: f64) -> RiskCheckResult {
        let portfolio = self.portfolio.read().await;
        let mut result = RiskCheckResult {
            passed: true,
            max_position_size_ok: true,
            daily_loss_limit_ok: true,
            drawdown_ok: true,
            concentration_ok: true,
            portfolio_heat_ok: true,
            warnings: Vec::new(),
        };

        // Max position size
        let max_pos_value = portfolio.equity * (self.config.max_position_size_pct / 100.0);
        if estimated_cost > max_pos_value {
            result.max_position_size_ok = false;
            result.passed = false;
            result.warnings.push(format!(
                "Position size ₹{:.2} exceeds max ₹{:.2} ({:.1}% of equity)",
                estimated_cost, max_pos_value, self.config.max_position_size_pct
            ));
        }

        // Daily loss limit
        let daily_loss_limit = portfolio.initial_balance * (self.config.max_daily_loss_pct / 100.0);
        if portfolio.daily_pnl < -daily_loss_limit {
            result.daily_loss_limit_ok = false;
            result.passed = false;
            result.warnings.push(format!(
                "Daily loss of ₹{:.2} exceeds limit of ₹{:.2} ({:.1}%)",
                portfolio.daily_pnl.abs(),
                daily_loss_limit,
                self.config.max_daily_loss_pct
            ));
        }

        // Drawdown
        let current_dd_pct = if portfolio.initial_balance > 0.0 {
            (portfolio.initial_balance - portfolio.equity) / portfolio.initial_balance * 100.0
        } else {
            0.0
        };
        if current_dd_pct > self.config.max_drawdown_pct {
            result.drawdown_ok = false;
            result.passed = false;
            result.warnings.push(format!(
                "Drawdown {:.1}% exceeds limit {:.1}%",
                current_dd_pct, self.config.max_drawdown_pct
            ));
        }

        // Concentration (total exposure in this symbol)
        let existing_exposure: f64 = portfolio
            .positions
            .iter()
            .filter(|p| p.symbol == symbol)
            .map(|p| p.entry_price * p.qty as f64)
            .sum();
        let total_exposure = existing_exposure + estimated_cost;
        let max_exposure = portfolio.equity * (self.config.max_concentration_pct / 100.0);
        if total_exposure > max_exposure {
            result.concentration_ok = false;
            result.passed = false;
            result.warnings.push(format!(
                "Concentration in {} ₹{:.2} exceeds max ₹{:.2} ({:.1}%)",
                symbol, total_exposure, max_exposure, self.config.max_concentration_pct
            ));
        }

        // Portfolio heat
        let total_risk: f64 = portfolio
            .positions
            .iter()
            .map(|p| {
                let risk_per_unit = match p.direction {
                    TradeDirection::Long => p.entry_price - p.stop_loss,
                    TradeDirection::Short => p.stop_loss - p.entry_price,
                };
                if risk_per_unit > 0.0 {
                    risk_per_unit * p.qty as f64
                } else {
                    0.0
                }
            })
            .sum();
        let max_heat = portfolio.equity * (self.config.max_portfolio_heat_pct / 100.0);
        if total_risk > max_heat {
            result.portfolio_heat_ok = false;
            result.passed = false;
            result.warnings.push(format!(
                "Portfolio heat ₹{:.2} exceeds max ₹{:.2} ({:.1}%)",
                total_risk, max_heat, self.config.max_portfolio_heat_pct
            ));
        }

        result
    }

    // ── Getters ───────────────────────────────────────────────────────────

    pub async fn get_summary(&self) -> PortfolioSummary {
        let p = self.portfolio.read().await;
        let win_rate = if p.total_trades > 0 {
            p.winning_trades as f64 / p.total_trades as f64 * 100.0
        } else {
            0.0
        };
        let margin_used: f64 = p
            .positions
            .iter()
            .map(|pos| pos.current_price * pos.qty as f64)
            .sum();
        let total_pnl_all_time = p.equity - p.initial_balance;

        PortfolioSummary {
            cash: p.cash,
            equity: p.equity,
            margin_used,
            free_margin: p.cash,
            daily_pnl: p.daily_pnl,
            daily_pnl_pct: if p.initial_balance > 0.0 {
                (p.daily_pnl / p.initial_balance) * 100.0
            } else {
                0.0
            },
            total_trades: p.total_trades,
            winning_trades: p.winning_trades,
            losing_trades: p.losing_trades,
            win_rate,
            consecutive_losses: p.consecutive_losses,
            max_drawdown: p.max_drawdown,
            max_drawdown_pct: p.max_drawdown_pct,
            open_positions: p.positions.len(),
            total_pnl_all_time,
        }
    }

    pub async fn get_positions(&self) -> Vec<Position> {
        let p = self.portfolio.read().await;
        p.positions.clone()
    }

    pub async fn get_position(&self, id: &str) -> Option<Position> {
        let p = self.portfolio.read().await;
        p.positions.iter().find(|pos| pos.id == id).cloned()
    }

    pub async fn get_trade_history(&self) -> Vec<ClosedTrade> {
        let p = self.portfolio.read().await;
        let mut history = p.trade_history.clone();
        history.reverse(); // Most recent first
        history
    }

    pub async fn get_recent_trades(&self, limit: usize) -> Vec<ClosedTrade> {
        let p = self.portfolio.read().await;
        let mut history = p.trade_history.clone();
        history.reverse();
        history.truncate(limit);
        history
    }
}

// ── BrokerAdapter Trait ──────────────────────────────────────────────────────

/// Unified interface for ALL broker types (paper AND live).
/// Every broker implementation shares the exact same API.
/// The frontend never knows whether it's talking to paper or live.
#[async_trait::async_trait]
pub trait BrokerAdapter: Send + Sync {
    /// Connect to the broker (authenticate, establish session)
    async fn connect(&self) -> Result<(), String>;

    /// Disconnect gracefully
    async fn disconnect(&self) -> Result<(), String>;

    /// Place an order. Returns the broker's order ID.
    async fn place_order(&self, request: OrderRequest, market_price: f64)
        -> Result<String, String>;

    /// Cancel an open order by ID
    async fn cancel_order(&self, order_id: &str) -> Result<(), String>;

    /// Get all open positions
    async fn get_positions(&self) -> Result<Vec<Position>, String>;

    /// Get portfolio summary
    async fn get_summary(&self) -> Result<PortfolioSummary, String>;

    /// Get order status
    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String>;

    /// Get recent trades
    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String>;

    /// Update all positions with latest market price. Returns closed trades.
    async fn update_price(
        &self,
        symbol: &str,
        market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String>;

    /// Close a position manually
    async fn close_position(
        &self,
        position_id: &str,
        exit_price: f64,
    ) -> Result<ClosedTrade, String>;

    /// Run risk checks before placing an order
    async fn check_risk(
        &self,
        symbol: &str,
        estimated_cost: f64,
    ) -> Result<RiskCheckResult, String>;

    /// Reset portfolio (paper only — no-op for live)
    async fn reset(&self) -> Result<(), String>;

    /// What mode are we in?
    fn mode(&self) -> TradingMode;

    /// Get a display name for this broker
    fn broker_name(&self) -> &str;
}

// ── PaperBroker Implementation ───────────────────────────────────────────────

/// Paper trading broker — wraps PaperEngine with the BrokerAdapter interface.
/// Uses virtual money but real market prices.
#[derive(Debug)]
pub struct PaperBroker {
    engine: Arc<PaperEngine>,
    connected: RwLock<bool>,
}

impl PaperBroker {
    pub fn new(config: PaperEngineConfig) -> Self {
        Self {
            engine: Arc::new(PaperEngine::new(config)),
            connected: RwLock::new(false),
        }
    }

    pub fn engine(&self) -> &Arc<PaperEngine> {
        &self.engine
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for PaperBroker {
    async fn connect(&self) -> Result<(), String> {
        let mut c = self.connected.write().await;
        *c = true;
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        let mut c = self.connected.write().await;
        *c = false;
        Ok(())
    }

    async fn place_order(
        &self,
        request: OrderRequest,
        market_price: f64,
    ) -> Result<String, String> {
        let connected = self.connected.read().await;
        if !*connected {
            return Err("Paper broker not connected. Call connect() first.".to_string());
        }
        drop(connected);

        // Risk check first
        let estimated_cost = market_price * request.qty as f64;
        let risk = self
            .engine
            .check_risk(&request.symbol, estimated_cost)
            .await;
        if !risk.passed {
            return Err(format!("Risk check failed: {}", risk.warnings.join("; ")));
        }

        self.engine.place_order(request, market_price).await
    }

    async fn cancel_order(&self, _order_id: &str) -> Result<(), String> {
        Err("Cancel not implemented for PaperBroker (orders fill instantly)".to_string())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        Ok(self.engine.get_positions().await)
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        Ok(self.engine.get_summary().await)
    }

    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String> {
        let portfolio = self.engine.portfolio.read().await;
        if let Some(order) = portfolio.orders.iter().find(|o| o.order_id == order_id) {
            Ok(order.status.clone())
        } else {
            Err(format!("Order {} not found", order_id))
        }
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        Ok(self.engine.get_recent_trades(limit).await)
    }

    async fn update_price(
        &self,
        symbol: &str,
        market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        Ok(self.engine.update_price(symbol, market_price).await)
    }

    async fn close_position(
        &self,
        position_id: &str,
        exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        self.engine.close_position(position_id, exit_price).await
    }

    async fn check_risk(
        &self,
        symbol: &str,
        estimated_cost: f64,
    ) -> Result<RiskCheckResult, String> {
        Ok(self.engine.check_risk(symbol, estimated_cost).await)
    }

    async fn reset(&self) -> Result<(), String> {
        self.engine.reset().await;
        Ok(())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Paper
    }

    fn broker_name(&self) -> &str {
        "Paper Trading"
    }
}

// ── Live Broker Stubs (removed) ──────────────────────────────────────────────
// ZerodhaKiteBroker and AngelOneBroker stubs were removed. They returned
// Err("not implemented") for every method — 100+ lines of dead code.
// Real broker adapters should be implemented in separate files when needed.

// ── BrokerRegistry ───────────────────────────────────────────────────────────

/// Manages broker instances and routes orders to the active one.
/// The frontend talks to the registry, never to individual brokers directly.
pub struct BrokerRegistry {
    paper: Arc<PaperBroker>,
    live_brokers: RwLock<Vec<Arc<dyn BrokerAdapter>>>,
    active_mode: RwLock<TradingMode>,
    active_broker_name: RwLock<String>,
}

impl std::fmt::Debug for BrokerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerRegistry")
            .field("active_mode", &self.active_mode)
            .field("active_broker_name", &self.active_broker_name)
            .field(
                "live_broker_count",
                &self.live_brokers.try_read().ok().map(|b| b.len()),
            )
            .finish()
    }
}

impl BrokerRegistry {
    pub fn new(config: PaperEngineConfig) -> Self {
        Self {
            paper: Arc::new(PaperBroker::new(config)),
            live_brokers: RwLock::new(Vec::new()),
            active_mode: RwLock::new(TradingMode::Paper),
            active_broker_name: RwLock::new("Paper Trading".to_string()),
        }
    }

    pub fn paper_engine(&self) -> Arc<PaperEngine> {
        self.paper.engine().clone()
    }

    /// Register a live broker implementation
    pub async fn register_live_broker(&self, broker: Arc<dyn BrokerAdapter>) {
        let mut brokers = self.live_brokers.write().await;
        brokers.push(broker);
    }

    /// Switch trading mode. Paper mode always available.
    /// Live mode requires at least one registered broker.
    pub async fn set_mode(&self, mode: TradingMode) -> Result<(), String> {
        match mode {
            TradingMode::Paper => {
                let mut m = self.active_mode.write().await;
                *m = TradingMode::Paper;
                let mut n = self.active_broker_name.write().await;
                *n = "Paper Trading".to_string();
                self.paper.connect().await?;
                Ok(())
            }
            TradingMode::Live => {
                let brokers = self.live_brokers.read().await;
                if brokers.is_empty() {
                    return Err(
                        "No live broker registered. Configure API credentials first.".to_string(),
                    );
                }
                // Disconnect paper broker first
                self.paper.disconnect().await?;
                // Try to connect the first live broker
                brokers[0].connect().await?;
                let mut m = self.active_mode.write().await;
                *m = TradingMode::Live;
                let mut n = self.active_broker_name.write().await;
                *n = brokers[0].broker_name().to_string();
                Ok(())
            }
        }
    }

    /// Get the currently active broker adapter
    pub async fn active_broker(&self) -> Arc<dyn BrokerAdapter> {
        let mode = self.active_mode.read().await;
        match *mode {
            TradingMode::Paper => self.paper.clone() as Arc<dyn BrokerAdapter>,
            TradingMode::Live => {
                let brokers = self.live_brokers.read().await;
                if brokers.is_empty() {
                    self.paper.clone() as Arc<dyn BrokerAdapter>
                } else {
                    brokers[0].clone()
                }
            }
        }
    }

    pub async fn current_mode(&self) -> TradingMode {
        *self.active_mode.read().await
    }

    pub async fn current_broker_name(&self) -> String {
        self.active_broker_name.read().await.clone()
    }
}
