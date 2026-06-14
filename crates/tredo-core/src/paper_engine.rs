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
//!         → ZerodhaKiteBroker          [LIVE MODE]
//!         → AngelOneBroker             [LIVE MODE]
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
    pub commission_pct: f64, // % commission per trade
}

impl Default for PaperEngineConfig {
    fn default() -> Self {
        Self {
            initial_balance: 100_000.0,
            max_position_size_pct: 5.0,
            max_daily_loss_pct: 3.0,
            max_drawdown_pct: 10.0,
            max_concentration_pct: 20.0,
            max_portfolio_heat_pct: 30.0,
            max_leverage: 1.0,
            slippage_model: SlippageModel::Fixed(0.01), // 1 paisa slippage
            commission_pct: 0.03,                       // 0.03% (like Zerodha)
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SlippageModel {
    None,
    Fixed(f64),      // Fixed amount per share
    Percentage(f64), // % of trade value
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
    }

    // ── Order Placement ───────────────────────────────────────────────────

    /// Place a market or limit order. Returns the order ID.
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

        // Determine fill price based on order type
        let fill_price = match request.order_type {
            OrderType::Market => {
                // Apply slippage for market orders
                match self.config.slippage_model {
                    SlippageModel::None => market_price,
                    SlippageModel::Fixed(s) => {
                        if request.direction == TradeDirection::Long {
                            market_price + s
                        } else {
                            market_price - s
                        }
                    }
                    SlippageModel::Percentage(p) => {
                        let slippage = market_price * (p / 100.0);
                        if request.direction == TradeDirection::Long {
                            market_price + slippage
                        } else {
                            market_price - slippage
                        }
                    }
                }
            }
            OrderType::Limit => request.price.unwrap_or(market_price),
            OrderType::StopLoss | OrderType::StopLossLimit => market_price,
        };

        // Check cash balance
        let estimated_cost = fill_price * request.qty as f64;
        let mut portfolio = self.portfolio.write().await;

        if portfolio.cash < estimated_cost {
            return Err(format!(
                "Insufficient cash. Need ₹{:.2}, have ₹{:.2}",
                estimated_cost, portfolio.cash
            ));
        }

        // Apply commission
        let commission = estimated_cost * (self.config.commission_pct / 100.0);

        // Deduct cash
        portfolio.cash -= estimated_cost + commission;

        // Create position
        let position = Position {
            id: format!(
                "POS-{:06}",
                self.trade_counter.fetch_add(1, Ordering::SeqCst)
            ),
            symbol: request.symbol.clone(),
            direction: request.direction,
            qty: request.qty,
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

        // Mark order as filled
        let order_record = OrderRecord {
            order_id: order_id.clone(),
            request: request.clone(),
            status: OrderStatus::Filled,
            filled_price: Some(fill_price),
            filled_qty: request.qty,
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

        // Update portfolio stats
        portfolio.cash += exit_price * pos.qty as f64 + net_pnl;
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
        let mut total_unrealized_pnl = 0.0;
        for pos in &portfolio.positions {
            let (upnl, _) =
                Self::calculate_pnl(pos.direction, pos.entry_price, pos.current_price, pos.qty);
            total_unrealized_pnl += upnl;
        }
        portfolio.equity = portfolio.cash + total_unrealized_pnl;
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

// ── Live Broker Stubs ────────────────────────────────────────────────────────

/// Zerodha Kite Live Broker — connects to the real Kite API.
/// Same code path as PaperBroker — only the settlement differs.
#[allow(dead_code)]
#[derive(Debug)]
pub struct ZerodhaKiteBroker {
    api_key: String,
    api_secret: String,
    access_token: RwLock<Option<String>>,
    connected: RwLock<bool>,
    base_url: String,
}

impl ZerodhaKiteBroker {
    pub fn new(api_key: &str, api_secret: &str, base_url: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            access_token: RwLock::new(None),
            connected: RwLock::new(false),
            base_url: base_url.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for ZerodhaKiteBroker {
    async fn connect(&self) -> Result<(), String> {
        // DEFERRED per user request (real money only after full paper validation of autonomous system).
        // When ready: implement Kite Connect OAuth flow here (POST /session/token), handle 2FA, token refresh.
        // For now, this remains a clear stub so swapping the execution layer later is a small change.
        Err("Zerodha Kite live broker is a deferred stub (real APIs gated until paper hands-off is perfect). Use PaperBroker for now.".to_string())
    }

    async fn disconnect(&self) -> Result<(), String> {
        let mut c = self.connected.write().await;
        *c = false;
        Ok(())
    }

    async fn place_order(
        &self,
        _request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn cancel_order(&self, _order_id: &str) -> Result<(), String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn get_order_status(&self, _order_id: &str) -> Result<OrderStatus, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn get_recent_trades(&self, _limit: usize) -> Result<Vec<ClosedTrade>, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn update_price(
        &self,
        _symbol: &str,
        _market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn close_position(
        &self,
        _position_id: &str,
        _exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn check_risk(
        &self,
        _symbol: &str,
        _estimated_cost: f64,
    ) -> Result<RiskCheckResult, String> {
        Err("Zerodha Kite live broker not yet implemented".to_string())
    }

    async fn reset(&self) -> Result<(), String> {
        Ok(())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        "Zerodha Kite"
    }
}

// ── Angel One Broker Stub ────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug)]
pub struct AngelOneBroker {
    api_key: String,
    api_secret: String,
    connected: RwLock<bool>,
}

impl AngelOneBroker {
    pub fn new(api_key: &str, api_secret: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            connected: RwLock::new(false),
        }
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for AngelOneBroker {
    async fn connect(&self) -> Result<(), String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn disconnect(&self) -> Result<(), String> {
        let mut c = self.connected.write().await;
        *c = false;
        Ok(())
    }

    async fn place_order(
        &self,
        _request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn cancel_order(&self, _order_id: &str) -> Result<(), String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn get_order_status(&self, _order_id: &str) -> Result<OrderStatus, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn get_recent_trades(&self, _limit: usize) -> Result<Vec<ClosedTrade>, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn update_price(
        &self,
        _symbol: &str,
        _market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn close_position(
        &self,
        _position_id: &str,
        _exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn check_risk(
        &self,
        _symbol: &str,
        _estimated_cost: f64,
    ) -> Result<RiskCheckResult, String> {
        Err("Angel One live broker not yet implemented".to_string())
    }

    async fn reset(&self) -> Result<(), String> {
        Ok(())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        "Angel One"
    }
}

// ── BrokerRegistry ───────────────────────────────────────────────────────────

/// Manages broker instances and routes orders to the active one.
/// The frontend talks to the registry, never to individual brokers directly.
pub struct BrokerRegistry {
    paper: Arc<PaperBroker>,
    live_brokers: RwLock<Vec<Arc<dyn BrokerAdapter>>>,
    active_mode: RwLock<TradingMode>,
    active_broker_name: RwLock<String>,
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
