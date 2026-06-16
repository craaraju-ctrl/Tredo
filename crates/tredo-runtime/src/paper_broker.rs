//! Paper broker — wraps tredo-core's PaperEngine into a unified execution interface.

use crate::risk_manager::RiskManager;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderSide {
    Buy,
    Sell,
}

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

    /// Broker name for logging.
    fn name(&self) -> &str;
}

/// Paper broker backed by the existing tredo-core PaperEngine.
pub struct PaperBroker {
    #[allow(dead_code)]
    risk_manager: RiskManager,
}

impl PaperBroker {
    pub fn new(risk_manager: RiskManager) -> Self {
        Self { risk_manager }
    }
}

#[async_trait]
impl ExecutionBroker for PaperBroker {
    async fn place_market_order(
        &self,
        _symbol: &str,
        _side: OrderSide,
        _quantity: f64,
    ) -> Result<OrderResult, Box<dyn std::error::Error + Send + Sync>> {
        // In a real implementation, this would call the existing paper_engine.rs.
        // For now, this is a placeholder that delegates to the autonomous pipeline.
        Ok(OrderResult {
            order_id: format!("paper-{}", chrono::Utc::now().timestamp_millis()),
            symbol: _symbol.to_string(),
            side: _side,
            filled_qty: _quantity,
            fill_price: 0.0,
            commission: 0.0,
            timestamp: chrono::Utc::now(),
        })
    }

    async fn get_cash_balance(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(100_000.0) // placeholder
    }

    async fn get_total_equity(&self) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(100_000.0) // placeholder
    }

    fn name(&self) -> &str {
        "PaperBroker"
    }
}
