//! Live broker — safety wrapper that gates real money execution.
//! Requires explicit confirmation before placing orders.

use crate::paper_broker::{ExecutionBroker, OrderResult, OrderSide};
use crate::risk_manager::RiskManager;
use async_trait::async_trait;
use tredo_core::paper_engine::{ClosedTrade, Position, PortfolioSummary, RiskCheckResult};

/// Safety gate for live trading — wraps any ExecutionBroker with safety checks.
pub struct LiveBrokerSafety<T: ExecutionBroker> {
    inner: T,
    risk_manager: RiskManager,
    require_per_trade_confirmation: bool,
}

impl<T: ExecutionBroker> LiveBrokerSafety<T> {
    pub fn new(inner: T, risk_manager: RiskManager, require_per_trade_confirmation: bool) -> Self {
        Self {
            inner,
            risk_manager,
            require_per_trade_confirmation,
        }
    }
}

#[async_trait]
impl<T: ExecutionBroker + Send + Sync> ExecutionBroker for LiveBrokerSafety<T> {
    async fn place_market_order(
        &self,
        symbol: &str,
        side: OrderSide,
        quantity: f64,
    ) -> Result<OrderResult, Box<dyn std::error::Error + Send + Sync>> {
        // Safety check 1: Hard stop
        if self.risk_manager.is_hard_stop_engaged() {
            return Err("Hard stop engaged — all live trading suspended".into());
        }

        // Safety check 2: Per-trade confirmation
        if self.require_per_trade_confirmation {
            eprintln!(
                "\n⚠ LIVE TRADE CONFIRMATION REQUIRED ⚠\n\
                 Symbol: {}\nSide: {:?}\nQty: {}\n\
                 Type 'YES' to confirm: ",
                symbol, side, quantity
            );
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            let input = input.trim().to_uppercase();
            if input != "YES" {
                return Err("Trade cancelled by user".into());
            }
        }

        self.inner.place_market_order(symbol, side, quantity).await
    }

    async fn get_cash_balance(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.get_cash_balance().await
    }

    async fn get_total_equity(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.get_total_equity().await
    }

    async fn get_positions(
        &self,
    ) -> Result<Vec<Position>, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.get_positions().await
    }

    async fn get_summary(
        &self,
    ) -> Result<PortfolioSummary, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.get_summary().await
    }

    async fn update_price_cache(&self, symbol: &str, price: f64) {
        self.inner.update_price_cache(symbol, price).await;
    }

    async fn monitor_positions(
        &self,
        symbol: &str,
        market_price: f64,
    ) -> Result<Vec<ClosedTrade>, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.monitor_positions(symbol, market_price).await
    }

    async fn check_risk(
        &self,
        symbol: &str,
        estimated_cost: f64,
    ) -> Result<RiskCheckResult, Box<dyn std::error::Error + Send + Sync>> {
        self.inner.check_risk(symbol, estimated_cost).await
    }

    async fn reset(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.inner.reset().await
    }

    fn name(&self) -> &str {
        "LiveBroker (safety-gated)"
    }
}
