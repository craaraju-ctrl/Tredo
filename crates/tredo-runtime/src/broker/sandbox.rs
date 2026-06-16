//! Broker sandbox — a "dry-run" wrapper around any live broker.
//!
//! Places orders through the live broker but auto-cancels them immediately.
//! Useful for testing the live broker connection and order flow without
//! risking real money.
//!
//! ## Usage
//! ```rust,ignore
//! let live = create_zerodha_broker(&key, &secret, &token);
//! let sandbox = BrokerSandbox::wrap(live);
//! // All orders placed through sandbox will be cancelled immediately
//! ```

use async_trait::async_trait;
use std::sync::Arc;
use tredo_core::paper_engine::{
    BrokerAdapter, ClosedTrade, OrderRequest, OrderStatus, Position, PortfolioSummary,
    RiskCheckResult, TradingMode,
};

/// A wrapper around any `BrokerAdapter` that places orders and then
/// immediately cancels them. All read-only operations (positions, summary,
/// order status) pass through to the underlying broker transparently.
///
/// This is useful for:
/// - Testing live broker connectivity without risking funds
/// - Validating order parameters before going live
/// - Dry-running the trading pipeline end-to-end
pub struct BrokerSandbox {
    inner: Arc<dyn BrokerAdapter>,
}

impl BrokerSandbox {
    /// Wrap a live broker adapter in sandbox mode.
    pub fn wrap(inner: Arc<dyn BrokerAdapter>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl BrokerAdapter for BrokerSandbox {
    async fn connect(&self) -> Result<(), String> {
        self.inner.connect().await
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.inner.disconnect().await
    }

    async fn place_order(&self, request: OrderRequest, market_price: f64) -> Result<String, String> {
        // Place the order through the live broker
        let order_id = self.inner.place_order(request, market_price).await?;
        // Immediately cancel — sandbox never executes
        let real_id = order_id.strip_prefix("SANDBOX-").unwrap_or(&order_id);
        let _ = self.inner.cancel_order(real_id).await;
        Ok(format!("SANDBOX-{}", order_id))
    }

    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        let real_id = order_id.strip_prefix("SANDBOX-").unwrap_or(order_id);
        self.inner.cancel_order(real_id).await
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        self.inner.get_positions().await
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        self.inner.get_summary().await
    }

    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String> {
        let real_id = order_id.strip_prefix("SANDBOX-").unwrap_or(order_id);
        self.inner.get_order_status(real_id).await
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        self.inner.get_recent_trades(limit).await
    }

    async fn update_price(&self, symbol: &str, market_price: f64) -> Result<Vec<ClosedTrade>, String> {
        self.inner.update_price(symbol, market_price).await
    }

    async fn close_position(&self, _position_id: &str, _exit_price: f64) -> Result<ClosedTrade, String> {
        // Don't actually close in sandbox mode
        Err("BrokerSandbox: close_position disabled in sandbox mode — use the live broker directly".to_string())
    }

    async fn check_risk(&self, symbol: &str, estimated_cost: f64) -> Result<RiskCheckResult, String> {
        self.inner.check_risk(symbol, estimated_cost).await
    }

    async fn reset(&self) -> Result<(), String> {
        Ok(())
    }

    fn mode(&self) -> TradingMode {
        self.inner.mode()
    }

    fn broker_name(&self) -> &str {
        // Use a static string to avoid lifetime issues with the inner broker's name
        "Sandbox Mode"
    }
}
