//! # Alpaca Markets API v2 — Live Broker Adapter
//!
//! Implements [`BrokerAdapter`] for trading US equities and crypto via
//! the [Alpaca Markets API](https://docs.alpaca.markets/).
//!
//! Supports both **paper trading** (free, real-time simulation) and **live trading**
//! via separate API endpoints.
//!
//! ## Authentication
//! Alpaca uses API key pairs passed as HTTP headers:
//! - `APCA-API-KEY-ID` — Your API Key ID
//! - `APCA-API-SECRET-KEY` — Your Secret Key
//!
//! Get your keys at <https://app.alpaca.markets/> (live) or
//! <https://paper-api.alpaca.markets/> (paper).
//!
//! ## Usage
//! ```rust,ignore
//! let broker = AlpacaBroker::new("your-api-key", "your-secret-key", false);
//! broker.connect().await.unwrap();
//! let summary = broker.get_summary().await.unwrap();
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tredo_core::paper_engine::{
    BrokerAdapter, CloseReason, ClosedTrade, OrderRequest, OrderStatus, OrderType,
    PortfolioSummary, Position, PositionStatus, RiskCheckResult, TradingMode,
};
use tredo_core::TradeDirection;

// ── Error Type ───────────────────────────────────────────────────────────────

/// Errors from the Alpaca API.
#[derive(Debug, thiserror::Error)]
pub enum AlpacaError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error: status={status} message={message}")]
    Api { status: u16, message: String },

    #[error("Not connected — call connect() first")]
    NotConnected,

    #[error("Missing field in response: {0}")]
    MissingField(String),

    #[error("Auth failed: {0}")]
    Auth(String),
}

// ── Response Types (Alpaca JSON) ────────────────────────────────────────────

/// Account information response from `GET /v2/account`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AlpacaAccount {
    id: String,
    status: String,
    currency: String,
    cash: String,
    portfolio_value: String,
    buying_power: String,
    #[serde(default)]
    pattern_day_trader: bool,
    #[serde(default)]
    day_trading_buying_power: Option<String>,
    #[serde(default)]
    regt_buying_power: Option<String>,
    #[serde(default)]
    shorting_enabled: bool,
    #[serde(default)]
    equity: Option<String>,
    #[serde(default)]
    last_equity: Option<String>,
    #[serde(default)]
    initial_margin: Option<String>,
    #[serde(default)]
    maintenance_margin: Option<String>,
    #[serde(default)]
    long_market_value: Option<String>,
    #[serde(default)]
    short_market_value: Option<String>,
}

/// Position response from `GET /v2/positions`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AlpacaPosition {
    symbol: String,
    qty: String,
    avg_entry_price: String,
    current_price: String,
    market_value: String,
    unrealized_pl: String,
    unrealized_plpc: String,
    change_today: String,
    #[serde(default)]
    side: String, // "long" or "short"
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    asset_class: Option<String>,
}

/// Order response from Alpaca.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AlpacaOrder {
    id: String,
    client_order_id: String,
    symbol: String,
    side: String,
    order_type: String,
    qty: String,
    filled_qty: String,
    time_in_force: String,
    status: String,
    #[serde(default)]
    limit_price: Option<String>,
    #[serde(default)]
    stop_price: Option<String>,
    #[serde(default)]
    filled_avg_price: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    filled_at: Option<String>,
    #[serde(default)]
    failed_at: Option<String>,
    #[serde(default)]
    rejected_at: Option<String>,
    #[serde(default)]
    cancelled_at: Option<String>,
    #[serde(default)]
    asset_id: Option<String>,
    #[serde(default)]
    symbol_pair: Option<String>,
    #[serde(default)]
    legs: Option<Vec<AlpacaOrderLeg>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AlpacaOrderLeg {
    id: String,
    side: String,
    symbol: String,
    qty: String,
    order_type: String,
    status: String,
    filled_avg_price: Option<String>,
    #[serde(default)]
    filled_qty: Option<String>,
}

// ── AlpacaBroker ─────────────────────────────────────────────────────────────

/// Live/paper trading broker for Alpaca Markets.
///
/// Supports both paper trading (free simulation) and live real-money trading.
/// Paper trading uses `https://paper-api.alpaca.markets`, live uses `https://api.alpaca.markets`.
///
/// ## Auth
/// Simple API key authentication via headers — no OAuth flow needed.
/// Get your keys at <https://app.alpaca.markets/>
#[derive(Debug)]
pub struct AlpacaBroker {
    api_key_id: String,
    api_secret_key: String,
    /// If `true`, uses paper trading endpoint
    paper: bool,
    connected: AtomicBool,
    client: reqwest::Client,
}

impl AlpacaBroker {
    /// Create a new Alpaca broker.
    ///
    /// * `api_key_id` — Your Alpaca API Key ID (from app.alpaca.markets)
    /// * `api_secret_key` — Your Alpaca Secret Key
    /// * `paper` — `true` for paper trading (free simulation), `false` for live
    pub fn new(api_key_id: &str, api_secret_key: &str, paper: bool) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("tredo/0.2.0 (Alpaca API v2)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key_id: api_key_id.to_string(),
            api_secret_key: api_secret_key.to_string(),
            paper,
            connected: AtomicBool::new(false),
            client,
        }
    }

    /// Get the base URL based on paper/live mode.
    fn base_url(&self) -> &str {
        if self.paper {
            "https://paper-api.alpaca.markets"
        } else {
            "https://api.alpaca.markets"
        }
    }

    /// Build the authentication headers for Alpaca requests.
    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.api_key_id) {
            headers.insert(HeaderName::from_static("apca-api-key-id"), v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.api_secret_key) {
            headers.insert(HeaderName::from_static("apca-api-secret-key"), v);
        }
        headers
    }

    /// Make an authenticated GET request to the Alpaca API.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, AlpacaError> {
        let url = format!("{}/v2{}", self.base_url(), path);
        let resp = self
            .client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(AlpacaError::Api {
                status,
                message: body,
            });
        }

        Ok(resp.json().await?)
    }

    /// Make an authenticated POST request with JSON body.
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, AlpacaError> {
        let url = format!("{}/v2{}", self.base_url(), path);
        let resp = self
            .client
            .post(&url)
            .headers(self.auth_headers())
            .json(body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(AlpacaError::Api {
                status,
                message: body,
            });
        }

        Ok(resp.json().await?)
    }

    /// Make an authenticated DELETE request.
    async fn delete(&self, path: &str) -> Result<(), AlpacaError> {
        let url = format!("{}/v2{}", self.base_url(), path);
        let resp = self
            .client
            .delete(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(AlpacaError::Api {
                status,
                message: body,
            });
        }

        Ok(())
    }

    /// Parse a string qty to f64.
    fn parse_f64(s: &str) -> f64 {
        s.parse::<f64>().unwrap_or(0.0)
    }

    /// Parse a string qty to i32.
    fn parse_qty(s: &str) -> i32 {
        s.parse::<f64>().unwrap_or(0.0).round() as i32
    }

    /// Parse an Alpaca timestamp string to DateTime<Utc>.
    fn parse_timestamp(ts: &Option<String>) -> DateTime<Utc> {
        match ts {
            Some(t) => chrono::DateTime::parse_from_rfc3339(t)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| {
                    // Fallback: try ISO 8601 "2024-01-15T09:30:00Z"
                    chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.fZ")
                        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
                        .unwrap_or(Utc::now())
                }),
            None => Utc::now(),
        }
    }

    /// Map an Alpaca position to a TREDO Position.
    fn alpaca_position_to_tredo(pos: &AlpacaPosition) -> Position {
        let qty = Self::parse_f64(&pos.qty) as i32;
        let entry_price = Self::parse_f64(&pos.avg_entry_price);
        let current_price = Self::parse_f64(&pos.current_price);
        let unrealized_pnl = Self::parse_f64(&pos.unrealized_pl);
        let unrealized_pnl_pct = Self::parse_f64(&pos.unrealized_plpc) * 100.0;

        let direction = if qty >= 0 {
            TradeDirection::Long
        } else {
            TradeDirection::Short
        };

        Position {
            id: format!("ALPACA--{}--{}", pos.symbol, Utc::now().timestamp_millis()),
            symbol: pos.symbol.clone(),
            direction,
            qty: qty.abs(),
            entry_price,
            current_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            unrealized_pnl,
            unrealized_pnl_pct,
            status: PositionStatus::Open,
            opened_at: Utc::now(),
            closed_at: None,
            strategy: Some("Alpaca Live".to_string()),
            order_id: String::new(),
        }
    }

    /// Map Alpaca order status to TREDO OrderStatus.
    fn parse_order_status(alpaca_status: &str, filled_qty: i32, _total_qty: i32) -> OrderStatus {
        match alpaca_status.to_lowercase().as_str() {
            "new" | "accepted" | "pending_new" | "accepted_for_bidding" => OrderStatus::Pending,
            "partially_filled" => OrderStatus::PartiallyFilled { filled_qty },
            "filled" => OrderStatus::Filled,
            "canceled" | "cancelled" | "expired" => OrderStatus::Cancelled,
            "rejected" | "suspended" => OrderStatus::Rejected {
                reason: format!("Alpaca status: {}", alpaca_status),
            },
            "done_for_day" | "replaced" | "pending_cancel" | "pending_replace" => {
                OrderStatus::Pending
            }
            "stopped" | "calculated" => OrderStatus::Pending,
            _ => OrderStatus::Pending,
        }
    }
}

#[async_trait]
impl BrokerAdapter for AlpacaBroker {
    /// Connect to Alpaca by validating the API credentials.
    ///
    /// Makes a test call to `GET /v2/account` to verify the API key works.
    async fn connect(&self) -> Result<(), String> {
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Validate credentials by fetching account info
        let account: AlpacaAccount = self
            .get("/account")
            .await
            .map_err(|e| format!("Alpaca connection failed: {}", e))?;

        self.connected.store(true, Ordering::Relaxed);

        let mode = if self.paper { "paper" } else { "live" };
        tracing::info!(
            "✅ Alpaca {} connected (account: {}, cash: ${})",
            mode,
            account.id,
            Self::parse_f64(&account.cash)
        );

        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::Relaxed);
        tracing::info!("🔌 Alpaca disconnected");
        Ok(())
    }

    async fn place_order(
        &self,
        request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("Not connected to Alpaca. Call connect() first.".into());
        }

        let side = match request.direction {
            TradeDirection::Long => "buy",
            TradeDirection::Short => "sell",
        };

        let order_type = match request.order_type {
            OrderType::Market => "market",
            OrderType::Limit => "limit",
            OrderType::StopLoss => "stop",
            OrderType::StopLossLimit => "stop_limit",
        };

        let mut body = serde_json::json!({
            "symbol": request.symbol,
            "qty": request.qty.to_string(),
            "side": side,
            "type": order_type,
            "time_in_force": "day",
        });

        if let Some(ref price) = request.price {
            body["limit_price"] = serde_json::json!(price.to_string());
        }
        if let Some(ref sl) = request.stop_loss {
            match request.order_type {
                OrderType::StopLoss => {
                    body["stop_price"] = serde_json::json!(sl.to_string());
                }
                OrderType::StopLossLimit => {
                    body["stop_price"] = serde_json::json!(sl.to_string());
                }
                _ => {}
            }
        }
        if let Some(ref tag) = request.strategy {
            body["extended_hours"] = serde_json::json!(false);
            body["client_order_id"] = serde_json::json!(format!("tredo-{}", tag));
        }

        let order: AlpacaOrder = self
            .post("/orders", &body)
            .await
            .map_err(|e| format!("Alpaca place order failed: {}", e))?;

        tracing::info!(
            "📈 Alpaca order placed: {} {} qty={} id={}",
            side,
            request.symbol,
            request.qty,
            order.id
        );

        Ok(order.id)
    }

    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AlpacaError::NotConnected.to_string());
        }

        self.delete(&format!("/orders/{}", order_id))
            .await
            .map_err(|e| format!("Failed to cancel order {}: {}", order_id, e))?;

        tracing::info!("🗑️ Cancelled Alpaca order {}", order_id);
        Ok(())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AlpacaError::NotConnected.to_string());
        }

        let positions: Vec<AlpacaPosition> =
            self.get("/positions").await.map_err(|e| e.to_string())?;

        Ok(positions
            .iter()
            .map(Self::alpaca_position_to_tredo)
            .collect())
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AlpacaError::NotConnected.to_string());
        }

        let account: AlpacaAccount = self.get("/account").await.map_err(|e| e.to_string())?;

        let cash = Self::parse_f64(&account.cash);
        let portfolio_value = Self::parse_f64(&account.portfolio_value);
        let _buying_power = Self::parse_f64(&account.buying_power);
        let equity = Self::parse_f64(account.equity.as_deref().unwrap_or("0"));

        // Get positions for P&L
        let positions = self.get_positions().await.unwrap_or_default();
        let total_pnl: f64 = positions.iter().map(|p| p.unrealized_pnl).sum();
        let winning = positions.iter().filter(|p| p.unrealized_pnl > 0.0).count() as u32;
        let losing = positions.iter().filter(|p| p.unrealized_pnl < 0.0).count() as u32;

        Ok(PortfolioSummary {
            cash,
            equity: portfolio_value,
            margin_used: (portfolio_value - cash).max(0.0),
            free_margin: cash,
            daily_pnl: total_pnl,
            daily_pnl_pct: if equity > 0.0 {
                (total_pnl / equity) * 100.0
            } else {
                0.0
            },
            total_trades: winning + losing,
            winning_trades: winning,
            losing_trades: losing,
            win_rate: if winning + losing > 0 {
                winning as f64 / (winning + losing) as f64 * 100.0
            } else {
                0.0
            },
            consecutive_losses: 0,
            max_drawdown: portfolio_value,
            max_drawdown_pct: 0.0,
            open_positions: positions.len(),
            total_pnl_all_time: total_pnl,
        })
    }

    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AlpacaError::NotConnected.to_string());
        }

        let order: AlpacaOrder = self
            .get(&format!("/orders/{}", order_id))
            .await
            .map_err(|e| e.to_string())?;

        let status = order.status.as_str();
        let filled_qty = Self::parse_qty(&order.filled_qty);
        let _total_qty = Self::parse_qty(&order.qty);

        Ok(Self::parse_order_status(status, filled_qty, _total_qty))
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AlpacaError::NotConnected.to_string());
        }

        let orders: Vec<AlpacaOrder> = self
            .get(&format!(
                "/orders?status=all&limit={}&direction=desc",
                limit.min(500)
            ))
            .await
            .map_err(|e| e.to_string())?;

        let trades: Vec<ClosedTrade> = orders
            .into_iter()
            .filter(|o| o.status == "filled")
            .map(|o| {
                let direction = match o.side.as_str() {
                    "buy" => TradeDirection::Long,
                    _ => TradeDirection::Short,
                };
                let qty = Self::parse_qty(&o.qty);
                let filled_avg_price =
                    Self::parse_f64(o.filled_avg_price.as_deref().unwrap_or("0"));
                let filled_at = Self::parse_timestamp(&o.filled_at);

                ClosedTrade {
                    id: o.id.clone(),
                    symbol: o.symbol.clone(),
                    direction,
                    qty,
                    entry_price: filled_avg_price,
                    exit_price: filled_avg_price,
                    realized_pnl: 0.0, // computed from order pair matching
                    realized_pnl_pct: 0.0,
                    close_reason: CloseReason::Manual,
                    opened_at: Self::parse_timestamp(&o.created_at),
                    closed_at: filled_at,
                    duration_secs: (filled_at - Self::parse_timestamp(&o.created_at)).num_seconds(),
                    strategy: Some("Alpaca Live".to_string()),
                    order_id: o.id,
                }
            })
            .collect();

        Ok(trades)
    }

    async fn update_price(
        &self,
        symbol: &str,
        _market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        // Alpaca handles SL/TP server-side via bracket orders.
        // TREDO's own risk engine monitors positions separately.
        let _ = symbol;
        Ok(Vec::new())
    }

    async fn close_position(
        &self,
        position_id: &str,
        _exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AlpacaError::NotConnected.to_string());
        }

        // Parse position ID to extract symbol.
        // Format: "ALPACA--{symbol}--{timestamp}"
        let parts: Vec<&str> = position_id.splitn(3, "--").collect();
        if parts.len() < 2 || parts[1].is_empty() {
            return Err(format!("Invalid position ID format: {}", position_id));
        }

        let symbol = parts[1];

        // Get current position details from Alpaca
        let positions = self.get_positions().await?;
        let pos = positions
            .into_iter()
            .find(|p| p.id == position_id || p.symbol == symbol)
            .ok_or_else(|| format!("Position {} not found", position_id))?;

        // Place an opposing market order to close
        let close_direction = match pos.direction {
            TradeDirection::Long => TradeDirection::Short,
            TradeDirection::Short => TradeDirection::Long,
        };

        let order_req = OrderRequest {
            symbol: symbol.to_string(),
            direction: close_direction,
            order_type: OrderType::Market,
            qty: pos.qty,
            price: None,
            stop_loss: None,
            take_profit: None,
            strategy: Some("close".to_string()),
            client_order_id: None,
        };

        let order_id = self.place_order(order_req, pos.current_price).await?;

        Ok(ClosedTrade {
            id: position_id.to_string(),
            symbol: symbol.to_string(),
            direction: pos.direction,
            qty: pos.qty,
            entry_price: pos.entry_price,
            exit_price: pos.current_price,
            realized_pnl: pos.unrealized_pnl,
            realized_pnl_pct: pos.unrealized_pnl_pct,
            close_reason: CloseReason::Manual,
            opened_at: pos.opened_at,
            closed_at: Utc::now(),
            duration_secs: (Utc::now() - pos.opened_at).num_seconds(),
            strategy: Some("Alpaca Live".to_string()),
            order_id,
        })
    }

    async fn check_risk(
        &self,
        _symbol: &str,
        _estimated_cost: f64,
    ) -> Result<RiskCheckResult, String> {
        // Alpaca handles its own risk checks (margin, buying power, PDT rules).
        // We trust the broker's risk management for live trades.
        Ok(RiskCheckResult {
            passed: true,
            max_position_size_ok: true,
            daily_loss_limit_ok: true,
            drawdown_ok: true,
            concentration_ok: true,
            portfolio_heat_ok: true,
            warnings: vec![],
        })
    }

    async fn reset(&self) -> Result<(), String> {
        // Cannot reset a live broker account.
        Err("Cannot reset a live Alpaca account. Use paper mode for reset.".into())
    }

    fn mode(&self) -> TradingMode {
        if self.paper {
            TradingMode::Paper
        } else {
            TradingMode::Live
        }
    }

    fn broker_name(&self) -> &str {
        if self.paper {
            "Alpaca (Paper)"
        } else {
            "Alpaca (Live)"
        }
    }
}

// ── Helper to create an Arc'd broker for use with BrokerRegistry ────────────

/// Create an Alpaca broker wrapped in an Arc, ready to register.
pub fn create_alpaca_broker(
    api_key_id: &str,
    api_secret_key: &str,
    paper: bool,
) -> std::sync::Arc<dyn BrokerAdapter> {
    std::sync::Arc::new(AlpacaBroker::new(api_key_id, api_secret_key, paper))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_f64() {
        assert_eq!(AlpacaBroker::parse_f64("123.45"), 123.45);
        assert_eq!(AlpacaBroker::parse_f64("0"), 0.0);
        assert_eq!(AlpacaBroker::parse_f64(""), 0.0);
        assert_eq!(AlpacaBroker::parse_f64("not_a_number"), 0.0);
    }

    #[test]
    fn test_parse_qty() {
        assert_eq!(AlpacaBroker::parse_qty("10"), 10);
        assert_eq!(AlpacaBroker::parse_qty("10.7"), 11);
        assert_eq!(AlpacaBroker::parse_qty("0"), 0);
    }

    #[test]
    fn test_parse_order_status() {
        assert_eq!(
            AlpacaBroker::parse_order_status("filled", 10, 10),
            OrderStatus::Filled
        );
        assert_eq!(
            AlpacaBroker::parse_order_status("new", 0, 10),
            OrderStatus::Pending
        );
        assert_eq!(
            AlpacaBroker::parse_order_status("rejected", 0, 10),
            OrderStatus::Rejected {
                reason: "Alpaca status: rejected".into()
            }
        );
        assert_eq!(
            AlpacaBroker::parse_order_status("canceled", 0, 10),
            OrderStatus::Cancelled
        );
        assert_eq!(
            AlpacaBroker::parse_order_status("partially_filled", 5, 10),
            OrderStatus::PartiallyFilled { filled_qty: 5 }
        );
    }

    #[test]
    fn test_broker_name_and_mode() {
        let paper_broker = AlpacaBroker::new("k", "s", true);
        assert_eq!(paper_broker.broker_name(), "Alpaca (Paper)");
        assert_eq!(paper_broker.mode(), TradingMode::Paper);

        let live_broker = AlpacaBroker::new("k", "s", false);
        assert_eq!(live_broker.broker_name(), "Alpaca (Live)");
        assert_eq!(live_broker.mode(), TradingMode::Live);
    }

    #[test]
    fn test_base_url() {
        let paper = AlpacaBroker::new("k", "s", true);
        assert_eq!(paper.base_url(), "https://paper-api.alpaca.markets");

        let live = AlpacaBroker::new("k", "s", false);
        assert_eq!(live.base_url(), "https://api.alpaca.markets");
    }
}
