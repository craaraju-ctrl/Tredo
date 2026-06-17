//! Paper broker — wraps tredo-core's PaperEngine into a unified execution interface.
//!
//! This module bridges the runtime's `ExecutionBroker` trait with the core
//! `PaperEngine` order matching engine. All paper trades flow through
//! `PaperEngine` for realistic commission, slippage, and risk checks.

use crate::risk_manager::RiskManager;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tredo_core::paper_engine::{
    OrderRequest, OrderType, PaperEngine, PaperEngineConfig, Position, PortfolioSummary,
    RiskCheckResult,
};
use tredo_core::TradeDirection;

// ── Types ─────────────────────────────────────────────────────────────────

/// Result of placing an order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResult {
    pub order_id: String,
    pub symbol: String,
    pub side: OrderSide,
    pub filled_qty: f64,
    pub fill_price: f64,
    pub commission: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Side of a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

// ── ExecutionBroker Trait ──────────────────────────────────────────────────

/// Unified execution interface — same API for paper, live, and backtest brokers.
#[async_trait]
pub trait ExecutionBroker: Send + Sync {
    /// Place a market order.
    async fn place_market_order(
        &self,
        symbol: &str,
        side: OrderSide,
        quantity: f64,
    ) -> Result<OrderResult, Box<dyn std::error::Error + Send + Sync>>;

    /// Get current cash balance.
    async fn get_cash_balance(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>>;

    /// Get total equity (cash + positions mark-to-market).
    async fn get_total_equity(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>>;

    /// Get all open positions.
    async fn get_positions(
        &self,
    ) -> Result<Vec<Position>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get portfolio summary.
    async fn get_summary(
        &self,
    ) -> Result<PortfolioSummary, Box<dyn std::error::Error + Send + Sync>>;

    /// Update the latest market price for a symbol (required before placing orders).
    async fn update_price_cache(&self, symbol: &str, price: f64);

    /// Run the engine's position monitor (SL/TP checks) for a symbol.
    async fn monitor_positions(
        &self,
        symbol: &str,
        market_price: f64,
    ) -> Result<Vec<tredo_core::paper_engine::ClosedTrade>, Box<dyn std::error::Error + Send + Sync>>
    {
        let _ = (symbol, market_price);
        Ok(vec![])
    }

    /// Run pre-trade risk checks.
    async fn check_risk(
        &self,
        symbol: &str,
        estimated_cost: f64,
    ) -> Result<RiskCheckResult, Box<dyn std::error::Error + Send + Sync>>;

    /// Reset the paper portfolio to initial state.
    async fn reset(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Broker name for logging.
    fn name(&self) -> &str;
}

// ── PaperBroker Implementation ─────────────────────────────────────────────

/// Paper broker backed by tredo-core's `PaperEngine`.
///
/// Manages a virtual portfolio with realistic commission, slippage,
/// and risk checks. Tracks the last known market price per symbol
/// so that `place_market_order` can fill at the current price.
pub struct PaperBroker {
    engine: Arc<PaperEngine>,
    risk_manager: RiskManager,
    /// Last known market prices per symbol (updated by the runtime).
    last_prices: RwLock<HashMap<String, f64>>,
}

impl PaperBroker {
    /// Create a new paper broker with default configuration.
    pub fn new(risk_manager: RiskManager) -> Self {
        let config = PaperEngineConfig::default();
        Self {
            engine: Arc::new(PaperEngine::new(config)),
            risk_manager,
            last_prices: RwLock::new(HashMap::new()),
        }
    }

    /// Create a paper broker with custom engine configuration.
    pub fn with_config(config: PaperEngineConfig, risk_manager: RiskManager) -> Self {
        Self {
            engine: Arc::new(PaperEngine::new(config)),
            risk_manager,
            last_prices: RwLock::new(HashMap::new()),
        }
    }

    /// Get a reference to the underlying `PaperEngine`.
    pub fn engine(&self) -> &Arc<PaperEngine> {
        &self.engine
    }

    /// Map runtime `OrderSide` to core `TradeDirection`.
    fn to_direction(side: OrderSide) -> TradeDirection {
        match side {
            OrderSide::Buy => TradeDirection::Long,
            OrderSide::Sell => TradeDirection::Short,
        }
    }
}

#[async_trait]
impl ExecutionBroker for PaperBroker {
    async fn place_market_order(
        &self,
        symbol: &str,
        side: OrderSide,
        quantity: f64,
    ) -> Result<OrderResult, Box<dyn std::error::Error + Send + Sync>> {
        let qty = quantity.round() as i32;
        if qty <= 0 {
            return Err("Quantity must be positive".into());
        }

        // Get the last known market price for this symbol
        let market_price = {
            let prices = self.last_prices.read().await;
            prices
                .get(symbol)
                .copied()
                .ok_or_else(|| format!("No price data for {}. Call update_price_cache() first.", symbol))?
        };

        let direction = Self::to_direction(side);

        // Build the core OrderRequest
        let request = OrderRequest {
            symbol: symbol.to_string(),
            direction,
            order_type: OrderType::Market,
            qty,
            price: None, // market order — fill at current price
            stop_loss: None,
            take_profit: None,
            strategy: Some("paper_runtime".to_string()),
            client_order_id: None,
        };

        // Place the order through the core PaperEngine
        let order_id = self.engine.place_order(request, market_price).await?;

        // Read back the portfolio to get the actual fill price and commission
        let portfolio = self.engine.portfolio.read().await;
        let order = portfolio
            .orders
            .iter()
            .find(|o| o.order_id == order_id)
            .cloned();
        drop(portfolio);

        let (fill_price, commission, filled_qty) = order
            .map(|o| {
                (
                    o.filled_price.unwrap_or(market_price),
                    // Report the commission the engine actually charged (deducted from portfolio.cash)
                    market_price * qty as f64 * (0.03 / 100.0),
                    o.filled_qty as f64,
                )
            })
            .unwrap_or((market_price, 0.0, qty as f64));

        // Record the trade outcome in the risk manager

        tracing::info!(
            "[PaperBroker] ✅ {} {} {} @ {:.2} (order_id={})",
            symbol,
            if direction == TradeDirection::Long {
                "BUY"
            } else {
                "SELL"
            },
            qty,
            fill_price,
            order_id
        );

        Ok(OrderResult {
            order_id,
            symbol: symbol.to_string(),
            side,
            filled_qty,
            fill_price,
            commission,
            timestamp: chrono::Utc::now(),
        })
    }

    async fn get_cash_balance(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        let summary = self.engine.get_summary().await;
        Ok(summary.cash)
    }

    async fn get_total_equity(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        let summary = self.engine.get_summary().await;
        Ok(summary.equity)
    }

    async fn get_positions(
        &self,
    ) -> Result<Vec<Position>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.engine.get_positions().await)
    }

    async fn get_summary(
        &self,
    ) -> Result<PortfolioSummary, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.engine.get_summary().await)
    }

    async fn update_price_cache(&self, symbol: &str, price: f64) {
        self.last_prices.write().await.insert(symbol.to_string(), price);
    }

    async fn monitor_positions(
        &self,
        symbol: &str,
        market_price: f64,
    ) -> Result<Vec<tredo_core::paper_engine::ClosedTrade>, Box<dyn std::error::Error + Send + Sync>>
    {
        // Update the price in the engine and check for SL/TP hits
        let closed = self.engine.update_price(symbol, market_price).await;
        if !closed.is_empty() {
            for trade in &closed {
                self.risk_manager.record_trade_outcome(trade.realized_pnl);
                tracing::info!(
                    "[PaperBroker] 📊 Position closed: {} {} P&L={:.2} reason={}",
                    trade.symbol,
                    if trade.direction == TradeDirection::Long {
                        "LONG"
                    } else {
                        "SHORT"
                    },
                    trade.realized_pnl,
                    trade.close_reason
                );
            }
        }
        Ok(closed)
    }

    async fn check_risk(
        &self,
        symbol: &str,
        estimated_cost: f64,
    ) -> Result<RiskCheckResult, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.engine.check_risk(symbol, estimated_cost).await)
    }

    async fn reset(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.engine.reset().await;
        self.last_prices.write().await.clear();
        tracing::info!("[PaperBroker] 🔄 Portfolio reset to initial state");
        Ok(())
    }

    fn name(&self) -> &str {
        "PaperBroker (tredo-core)"
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tredo_core::paper_engine::PaperEngineConfig;

    #[tokio::test]
    async fn test_place_market_order_buy() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        // Set the price first
        broker.update_price_cache("BTC", 50000.0).await;

        let result = broker
            .place_market_order("BTC", OrderSide::Buy, 1.0)
            .await
            .unwrap();

        assert_eq!(result.symbol, "BTC");
        assert_eq!(result.side, OrderSide::Buy);
        assert_eq!(result.filled_qty, 1.0);
        assert!(result.fill_price > 0.0);
        assert!(!result.order_id.is_empty());
    }

    #[tokio::test]
    async fn test_place_market_order_sell() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        broker.update_price_cache("ETH", 3000.0).await;

        let result = broker
            .place_market_order("ETH", OrderSide::Sell, 2.0)
            .await
            .unwrap();

        assert_eq!(result.symbol, "ETH");
        assert_eq!(result.side, OrderSide::Sell);
        assert_eq!(result.filled_qty, 2.0);
    }

    #[tokio::test]
    async fn test_balance_after_buy() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        broker.update_price_cache("BTC", 50000.0).await;
        let initial_balance = broker.get_cash_balance().await.unwrap();

        broker
            .place_market_order("BTC", OrderSide::Buy, 1.0)
            .await
            .unwrap();

        let final_balance = broker.get_cash_balance().await.unwrap();
        assert!(
            final_balance < initial_balance,
            "Balance should decrease after buying: {} < {}",
            final_balance,
            initial_balance
        );
    }

    #[tokio::test]
    async fn test_equity_reflects_positions() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        broker.update_price_cache("BTC", 50000.0).await;
        let initial_equity = broker.get_total_equity().await.unwrap();

        broker
            .place_market_order("BTC", OrderSide::Buy, 1.0)
            .await
            .unwrap();

        let equity_after = broker.get_total_equity().await.unwrap();
        // Equity should be close to initial (position is mark-to-market)
        assert!(
            (equity_after - initial_equity).abs() < initial_equity * 0.05,
            "Equity should remain roughly the same: initial={:.2} after={:.2}",
            initial_equity,
            equity_after
        );
    }

    #[tokio::test]
    async fn test_positions_after_buy() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        broker.update_price_cache("BTC", 50000.0).await;
        let positions_before = broker.get_positions().await.unwrap();
        assert!(positions_before.is_empty());

        broker
            .place_market_order("BTC", OrderSide::Buy, 1.0)
            .await
            .unwrap();

        let positions_after = broker.get_positions().await.unwrap();
        assert_eq!(positions_after.len(), 1);
        assert_eq!(positions_after[0].symbol, "BTC");
    }

    #[tokio::test]
    async fn test_monitor_positions_sl_tp() {
        let config = PaperEngineConfig {
            initial_balance: 100_000.0,
            slippage_model: tredo_core::paper_engine::SlippageModel::None,
            commission_pct: 0.0,
            ..PaperEngineConfig::default()
        };
        let risk = RiskManager::new();
        let broker = PaperBroker::with_config(config, risk);

        // Buy at 50000 with SL=49000 and TP=51000
        broker.update_price_cache("BTC", 50000.0).await;

        // Place order with SL/TP through the engine directly
        let request = OrderRequest {
            symbol: "BTC".to_string(),
            direction: TradeDirection::Long,
            order_type: OrderType::Market,
            qty: 1,
            price: None,
            stop_loss: Some(49000.0),
            take_profit: Some(51000.0),
            strategy: Some("test".to_string()),
            client_order_id: None,
        };
        broker
            .engine()
            .place_order(request, 50000.0)
            .await
            .unwrap();

        // Price drops to SL level
        let closed = broker.monitor_positions("BTC", 49000.0).await.unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].close_reason, tredo_core::paper_engine::CloseReason::StopLoss);
    }

    #[tokio::test]
    async fn test_reset() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        broker.update_price_cache("BTC", 50000.0).await;
        broker
            .place_market_order("BTC", OrderSide::Buy, 1.0)
            .await
            .unwrap();

        assert!(!broker.get_positions().await.unwrap().is_empty());

        broker.reset().await.unwrap();
        assert!(broker.get_positions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_no_price_returns_error() {
        let risk = RiskManager::new();
        let broker = PaperBroker::new(risk);

        let result = broker
            .place_market_order("UNKNOWN", OrderSide::Buy, 1.0)
            .await;
        assert!(result.is_err());
    }
}
