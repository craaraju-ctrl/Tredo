//! # Upstox API v2 — Free Indian Discount Broker Adapter
//!
//! Implements [`BrokerAdapter`] for trading via the Upstox REST API v2.
//! Upstox is a popular free Indian discount broker with a developer-friendly API.
//!
//! ## Authentication Flow
//! 1. Register app at https://upstox.com/developer/ to get `client_id` + `client_secret`
//! 2. User authorizes via: `https://api.upstox.com/v2/login/authorization/dialog?client_id={CLIENT_ID}&redirect_uri={REDIRECT_URI}`
//! 3. Upstox redirects to callback URL with a `code` parameter
//! 4. Exchange `code` for an `access_token` (valid until revoked)
//! 5. All subsequent requests use `Authorization: Bearer {access_token}`
//!
//! ## Usage
//! See the `BrokerAdapter` trait for the full API.
//!
//! ## Environment Variables
//! - `UPSTOX_CLIENT_ID` — Your API client ID
//! - `UPSTOX_CLIENT_SECRET` — Your API client secret
//! - `UPSTOX_REDIRECT_URI` — OAuth redirect URI
//! - `UPSTOX_ACCESS_TOKEN` — Pre-obtained access token (skip login flow)
//! - `UPSTOX_SANDBOX` — Set to "true" to use sandbox environment

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tredo_core::paper_engine::{
    BrokerAdapter, CloseReason, ClosedTrade, OrderRequest, OrderStatus, OrderType,
    PortfolioSummary, Position, PositionStatus, RiskCheckResult, TradingMode,
};
use tredo_core::TradeDirection;

// ── Error Type ───────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum UpstoxError {
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

// ── Response Types ───────────────────────────────────────────────────────────    /// Upstox API response envelope.
#[derive(Debug, Deserialize)]
struct UpstoxResponse<T> {
    status: String,
    data: Option<T>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    errors: Option<Vec<UpstoxErrorItem>>,
}

#[derive(Debug, Deserialize)]
struct UpstoxErrorItem {
    #[serde(default)]
    message: String,
}

/// Token response from /login/authorization/token
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// User profile / margin response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UpstoxMarginData {
    #[serde(default)]
    equity: Option<MarginSegment>,
    #[serde(default)]
    commodity: Option<MarginSegment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct MarginSegment {
    #[serde(default)]
    available_margin: f64,
    #[serde(default)]
    used_margin: f64,
    #[serde(default)]
    payin_amount: f64,
    #[serde(default)]
    adhoc_margin: f64,
    #[serde(default)]
    collateral: f64,
}

/// Order placement request
#[derive(Debug, Serialize)]
struct PlaceOrderRequest {
    quantity: i32,
    product: String,
    validity: String,
    price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    instrument_token: String,
    order_type: String,
    transaction_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    disclosed_quantity: Option<i32>,
    trigger_price: f64,
    is_amo: bool,
}

/// Order response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct OrderResponse {
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    order_status: Option<String>,
}

/// Position response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UpstoxPosition {
    #[serde(default)]
    trading_symbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    instrument_token: Option<String>,
    #[serde(default)]
    quantity: Option<i32>,
    #[serde(default)]
    average_price: Option<f64>,
    #[serde(default)]
    last_price: Option<f64>,
    #[serde(default)]
    pnl: Option<f64>,
    #[serde(default)]
    day_buy_quantity: Option<i32>,
    #[serde(default)]
    day_sell_quantity: Option<i32>,
    #[serde(default)]
    buy_price: Option<f64>,
    #[serde(default)]
    sell_price: Option<f64>,
    #[serde(default)]
    multiplier: Option<f64>,
    #[serde(default)]
    buy_amount: Option<f64>,
    #[serde(default)]
    sell_amount: Option<f64>,
}

/// Holding response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UpstoxHolding {
    #[serde(default)]
    trading_symbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    isin: Option<String>,
    #[serde(default)]
    t1_quantity: Option<i32>,
    #[serde(default)]
    quantity: Option<i32>,
    #[serde(default)]
    average_price: Option<f64>,
    #[serde(default)]
    last_price: Option<f64>,
    #[serde(default)]
    pnl: Option<f64>,
    #[serde(default)]
    haircut: Option<f64>,
}

/// Order details for history/trade book
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UpstoxOrder {
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    trading_symbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    transaction_type: Option<String>,
    #[serde(default)]
    order_type: Option<String>,
    #[serde(default)]
    quantity: Option<f64>,
    #[serde(default)]
    filled_quantity: Option<f64>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    average_price: Option<f64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    filled_at: Option<String>,
    #[serde(default)]
    order_timestamp: Option<String>,
    #[serde(default)]
    status_message: Option<String>,
    #[serde(default)]
    tag: Option<String>,
}

// ── UpstoxBroker ──────────────────────────────────────────────────────────────

/// Live trading broker for Upstox API v2.
///
/// Uses the [Upstox API v2](https://upstox.com/developer/api-documentation/) to
/// place real-money orders, fetch positions, and manage portfolio state.
#[allow(dead_code)]
pub struct UpstoxBroker {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    base_url: String,
    access_token: RwLock<Option<String>>,
    connected: AtomicBool,
    client: reqwest::Client,
    sandbox: bool,
}

impl std::fmt::Debug for UpstoxBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstoxBroker")
            .field("base_url", &self.base_url)
            .field("connected", &self.connected)
            .field("sandbox", &self.sandbox)
            .finish()
    }
}

impl UpstoxBroker {
    /// Create a new Upstox broker.
    ///
    /// * `client_id` — Your Upstox client ID (from developer portal)
    /// * `client_secret` — Your Upstox client secret
    /// * `redirect_uri` — OAuth redirect URI
    /// * `access_token` — Pre-obtained access token (or empty string to get via connect())
    /// * `sandbox` — Whether to use sandbox environment
    pub fn new(
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        access_token: &str,
        sandbox: bool,
    ) -> Self {
        let base_url = if sandbox {
            "https://api-sandbox.upstox.com/v2"
        } else {
            "https://api.upstox.com/v2"
        };

        let client = reqwest::Client::builder()
            .user_agent("tredo/0.2.0 (Upstox API v2)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            redirect_uri: redirect_uri.to_string(),
            base_url: base_url.to_string(),
            access_token: RwLock::new(if access_token.is_empty() {
                None
            } else {
                Some(access_token.to_string())
            }),
            connected: AtomicBool::new(false),
            client,
            sandbox,
        }
    }

    /// Set a pre-obtained access token directly.
    pub async fn set_access_token(&self, token: &str) {
        let mut t = self.access_token.write().await;
        *t = Some(token.to_string());
    }

    /// Get the authorization URL for the OAuth flow.
    pub fn auth_url(&self) -> String {
        format!(
            "https://api.upstox.com/v2/login/authorization/dialog?client_id={}&redirect_uri={}&response_type=code",
            self.client_id, self.redirect_uri
        )
    }

    /// Build auth header.
    async fn auth_header(&self) -> Result<String, UpstoxError> {
        let token = self.access_token.read().await;
        match token.as_ref() {
            Some(t) => Ok(format!("Bearer {}", t)),
            None => Err(UpstoxError::Auth(
                "No access token. Call connect(code) first or set via set_access_token().".into(),
            )),
        }
    }

    /// Make an authenticated GET request.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, UpstoxError> {
        let url = format!("{}{}", self.base_url, path);
        let auth = self.auth_header().await?;

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(UpstoxError::Api {
                status,
                message: body,
            });
        }

        let envelope: UpstoxResponse<T> = resp.json().await?;
        if envelope.status != "success" && envelope.status != "ok" {
            let err_msg = envelope
                .errors
                .and_then(|e| e.into_iter().next().map(|e| e.message))
                .or(envelope.message)
                .unwrap_or_else(|| "Unknown error".to_string());
            return Err(UpstoxError::Api {
                status,
                message: err_msg,
            });
        }

        envelope
            .data
            .ok_or_else(|| UpstoxError::MissingField("data".into()))
    }

    /// Make an authenticated POST request with JSON body.
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, UpstoxError> {
        let url = format!("{}{}", self.base_url, path);
        let auth = self.auth_header().await?;

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(UpstoxError::Api {
                status,
                message: body,
            });
        }

        let envelope: UpstoxResponse<T> = resp.json().await?;
        if envelope.status != "success" && envelope.status != "ok" {
            return Err(UpstoxError::Api {
                status,
                message: envelope.message.unwrap_or_else(|| "Unknown error".into()),
            });
        }

        envelope
            .data
            .ok_or_else(|| UpstoxError::MissingField("data".into()))
    }

    /// Make a DELETE request.
    async fn delete<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, UpstoxError> {
        let url = format!("{}{}", self.base_url, path);
        let auth = self.auth_header().await?;

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", &auth)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(UpstoxError::Api {
                status,
                message: body,
            });
        }

        let envelope: UpstoxResponse<T> = resp.json().await?;
        if envelope.status != "success" && envelope.status != "ok" {
            return Err(UpstoxError::Api {
                status,
                message: envelope.message.unwrap_or_else(|| "Unknown error".into()),
            });
        }

        envelope
            .data
            .ok_or_else(|| UpstoxError::MissingField("data".into()))
    }

    /// Map TREDO OrderType to Upstox order type string.
    fn upstox_order_type(order_type: OrderType) -> &'static str {
        match order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopLoss => "SL",
            OrderType::StopLossLimit => "SL-M",
        }
    }

    /// Map TREDO TradeDirection to Upstox transaction type.
    fn upstox_transaction_type(direction: TradeDirection) -> &'static str {
        match direction {
            TradeDirection::Long => "BUY",
            TradeDirection::Short => "SELL",
        }
    }

    /// Map Upstox order status to TREDO OrderStatus.
    fn parse_order_status(status: &str, filled_qty: i32, total_qty: i32) -> OrderStatus {
        match status.to_uppercase().as_str() {
            "PENDING" | "OPEN" | "VALIDATION_PENDING" | "TRIGGER_PENDING" => OrderStatus::Pending,
            "COMPLETE" | "FILLED" => {
                if filled_qty >= total_qty {
                    OrderStatus::Filled
                } else {
                    OrderStatus::PartiallyFilled { filled_qty }
                }
            }
            "CANCELLED" | "CANCELED" => OrderStatus::Cancelled,
            "REJECTED" | "FAILED" => OrderStatus::Rejected {
                reason: "Order rejected by Upstox".into(),
            },
            _ => OrderStatus::Pending,
        }
    }

    /// Map Upstox product type string.
    #[allow(dead_code)]
    fn product_type(product: &str) -> &str {
        match product {
            "D" => "D",
            "I" => "I",
            "M" | "MIS" => "M",
            "CNC" => "C",
            _ => "D", // Default to delivery
        }
    }
}

#[async_trait]
impl BrokerAdapter for UpstoxBroker {
    /// Exchange authorization code for access token, or validate existing token.
    async fn connect(&self) -> Result<(), String> {
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        // If we already have a token, just validate it by making a test call
        {
            let token = self.access_token.read().await;
            if token.is_some() {
                // Try a simple API call to validate
                let url = format!("{}/user/get-profile", self.base_url);
                let auth = format!("Bearer {}", token.as_ref().unwrap());
                drop(token);

                let resp = self
                    .client
                    .get(&url)
                    .header("Authorization", &auth)
                    .header("Accept", "application/json")
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;

                if let Ok(r) = resp {
                    if r.status().is_success() {
                        self.connected.store(true, Ordering::Relaxed);
                        tracing::info!("✅ Upstox connected (existing token)");
                        return Ok(());
                    }
                }
            }
        }

        // No valid token — provide instructions
        Err(format!(
            "Upstox: No valid access token. Set UPSTOX_ACCESS_TOKEN or visit:\n  {}",
            self.auth_url()
        ))
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::Relaxed);
        let mut token = self.access_token.write().await;
        *token = None;
        tracing::info!("🔌 Upstox disconnected");
        Ok(())
    }

    async fn place_order(
        &self,
        request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("Not connected to Upstox. Call connect() first.".into());
        }

        // Build instrument token — use symbol as instrument name
        // In production, you'd resolve the instrument_token from a master contract list
        let instrument_token = format!("NSE_EQ|{}", request.symbol);

        let order_req = PlaceOrderRequest {
            quantity: request.qty,
            product: "D".to_string(), // Delivery by default
            validity: "DAY".to_string(),
            price: match request.order_type {
                OrderType::Market => 0.0,
                _ => request.price.unwrap_or(0.0),
            },
            tag: request.strategy.clone(),
            instrument_token,
            order_type: Self::upstox_order_type(request.order_type).to_string(),
            transaction_type: Self::upstox_transaction_type(request.direction).to_string(),
            disclosed_quantity: None,
            trigger_price: request.stop_loss.unwrap_or(0.0),
            is_amo: false,
        };

        let body =
            serde_json::to_value(&order_req).map_err(|e| format!("Serialization error: {}", e))?;

        let order_resp: OrderResponse = self
            .post("/order/place", &body)
            .await
            .map_err(|e| format!("Upstox place order failed: {}", e))?;

        let order_id = order_resp
            .order_id
            .ok_or_else(|| "No order_id in response".to_string())?;

        tracing::info!(
            "📈 Upstox order placed: {} {} qty={} id={}",
            Self::upstox_transaction_type(request.direction),
            request.symbol,
            request.qty,
            order_id
        );

        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(UpstoxError::NotConnected.to_string());
        }

        let _: serde_json::Value = self
            .delete(&format!("/order/cancel?order_id={}", order_id))
            .await
            .map_err(|e| format!("Failed to cancel order {}: {}", order_id, e))?;

        tracing::info!("🗑️ Cancelled Upstox order {}", order_id);
        Ok(())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(UpstoxError::NotConnected.to_string());
        }

        let positions: Vec<UpstoxPosition> = self
            .get("/portfolio/get-positions")
            .await
            .map_err(|e| e.to_string())?;

        let holdings: Vec<UpstoxHolding> = self
            .get("/portfolio/get-holdings")
            .await
            .map_err(|e| e.to_string())?;

        let now = Utc::now();
        let mut result = Vec::new();

        for pos in &positions {
            let qty = pos.quantity.unwrap_or(0);
            if qty == 0 {
                continue;
            }
            let direction = if qty > 0 {
                TradeDirection::Long
            } else {
                TradeDirection::Short
            };
            let entry_price = pos.average_price.unwrap_or(0.0);
            let current_price = pos.last_price.unwrap_or(entry_price);
            let pnl = pos.pnl.unwrap_or(0.0);
            let symbol = pos
                .trading_symbol
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            result.push(Position {
                id: format!("UPSTOX--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction,
                qty: qty.abs(),
                entry_price,
                current_price,
                stop_loss: 0.0,
                take_profit: 0.0,
                unrealized_pnl: pnl,
                unrealized_pnl_pct: if entry_price > 0.0 {
                    (current_price - entry_price) / entry_price * 100.0
                } else {
                    0.0
                },
                status: PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("Upstox Live".to_string()),
                order_id: String::new(),
            });
        }

        // Add holdings (delivery positions)
        for h in &holdings {
            let qty = h.quantity.unwrap_or(0);
            if qty == 0
                || result
                    .iter()
                    .any(|p| p.symbol == h.trading_symbol.as_deref().unwrap_or(""))
            {
                continue;
            }
            let symbol = h
                .trading_symbol
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            result.push(Position {
                id: format!("UPSTOX-HLDG--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction: TradeDirection::Long,
                qty,
                entry_price: h.average_price.unwrap_or(0.0),
                current_price: h.last_price.unwrap_or(0.0),
                stop_loss: 0.0,
                take_profit: 0.0,
                unrealized_pnl: h.pnl.unwrap_or(0.0),
                unrealized_pnl_pct: 0.0,
                status: PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("Upstox Holdings".to_string()),
                order_id: String::new(),
            });
        }

        Ok(result)
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(UpstoxError::NotConnected.to_string());
        }

        let margin: UpstoxMarginData = self
            .get("/user/get-margin-and-balance")
            .await
            .map_err(|e| e.to_string())?;

        let equity = margin.equity.unwrap_or(MarginSegment {
            available_margin: 0.0,
            used_margin: 0.0,
            payin_amount: 0.0,
            adhoc_margin: 0.0,
            collateral: 0.0,
        });

        let cash = equity.available_margin + equity.adhoc_margin + equity.collateral;
        let used = equity.used_margin;
        let total_equity = cash + used;

        let positions = self.get_positions().await.unwrap_or_default();
        let total_pnl: f64 = positions.iter().map(|p| p.unrealized_pnl).sum();
        let winning = positions.iter().filter(|p| p.unrealized_pnl > 0.0).count() as u32;
        let losing = positions.iter().filter(|p| p.unrealized_pnl < 0.0).count() as u32;

        Ok(PortfolioSummary {
            cash,
            equity: total_equity,
            margin_used: used,
            free_margin: cash,
            daily_pnl: total_pnl,
            daily_pnl_pct: if total_equity > 0.0 {
                (total_pnl / total_equity) * 100.0
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
            max_drawdown: total_equity,
            max_drawdown_pct: 0.0,
            open_positions: positions.len(),
            total_pnl_all_time: total_pnl,
        })
    }

    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(UpstoxError::NotConnected.to_string());
        }

        let order: UpstoxOrder = self
            .get(&format!("/order/details?order_id={}", order_id))
            .await
            .map_err(|e| e.to_string())?;

        let status = order.status.as_deref().unwrap_or("PENDING");
        let filled_qty = order.filled_quantity.unwrap_or(0.0).round() as i32;
        let total_qty = order.quantity.unwrap_or(0.0).round() as i32;

        Ok(Self::parse_order_status(status, filled_qty, total_qty))
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(UpstoxError::NotConnected.to_string());
        }

        let orders: Vec<UpstoxOrder> = self
            .get("/order/get-trades")
            .await
            .map_err(|e| e.to_string())?;

        let now = Utc::now();
        let mut trades: Vec<ClosedTrade> = orders
            .into_iter()
            .filter(|o| {
                o.status.as_deref() == Some("COMPLETE") || o.status.as_deref() == Some("FILLED")
            })
            .take(limit)
            .map(|o| {
                let direction = match o.transaction_type.as_deref() {
                    Some("BUY") => TradeDirection::Long,
                    _ => TradeDirection::Short,
                };
                let qty = (o.filled_quantity.unwrap_or(0.0).round() as i32).abs();
                let price = o.average_price.unwrap_or(0.0);

                ClosedTrade {
                    id: o.order_id.unwrap_or_default(),
                    symbol: o.trading_symbol.unwrap_or_default(),
                    direction,
                    qty,
                    entry_price: price,
                    exit_price: 0.0,
                    realized_pnl: 0.0,
                    realized_pnl_pct: 0.0,
                    close_reason: CloseReason::Manual,
                    opened_at: now,
                    closed_at: now,
                    duration_secs: 0,
                    strategy: Some("Upstox Live".to_string()),
                    order_id: String::new(),
                }
            })
            .collect();

        trades.reverse();
        Ok(trades)
    }

    async fn update_price(
        &self,
        _symbol: &str,
        _market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        Ok(Vec::new())
    }

    async fn close_position(
        &self,
        position_id: &str,
        _exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(UpstoxError::NotConnected.to_string());
        }

        let parts: Vec<&str> = position_id.splitn(3, "--").collect();
        if parts.len() < 2 || parts[1].is_empty() {
            return Err(format!("Invalid position ID format: {}", position_id));
        }

        let symbol = parts[1];
        let positions = self.get_positions().await?;
        let pos = positions
            .into_iter()
            .find(|p| p.id == position_id)
            .ok_or_else(|| format!("Position {} not found", position_id))?;

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
            opened_at: Utc::now(),
            closed_at: Utc::now(),
            duration_secs: 0,
            strategy: Some("Upstox Live".to_string()),
            order_id,
        })
    }

    async fn check_risk(
        &self,
        _symbol: &str,
        _estimated_cost: f64,
    ) -> Result<RiskCheckResult, String> {
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
        Err("Cannot reset a live Upstox account. Use paper mode.".into())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        if self.sandbox {
            "Upstox (Sandbox)"
        } else {
            "Upstox"
        }
    }
}

// ── Helper to create an Arc'd broker ─────────────────────────────────────────

pub fn create_upstox_broker(
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    access_token: &str,
) -> std::sync::Arc<dyn BrokerAdapter> {
    let sandbox = std::env::var("UPSTOX_SANDBOX")
        .map(|v| v == "true")
        .unwrap_or(false);
    std::sync::Arc::new(UpstoxBroker::new(
        client_id,
        client_secret,
        redirect_uri,
        access_token,
        sandbox,
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broker_name_and_mode() {
        let broker = UpstoxBroker::new("cid", "cs", "http://localhost", "", false);
        assert_eq!(broker.broker_name(), "Upstox");
        assert_eq!(broker.mode(), TradingMode::Live);
    }

    #[test]
    fn test_sandbox_name() {
        let broker = UpstoxBroker::new("cid", "cs", "http://localhost", "", true);
        assert_eq!(broker.broker_name(), "Upstox (Sandbox)");
    }

    #[test]
    fn test_auth_url() {
        let broker = UpstoxBroker::new(
            "my_client_id",
            "sec",
            "http://localhost:8080/callback",
            "",
            false,
        );
        let url = broker.auth_url();
        assert!(url.contains("my_client_id"));
        assert!(url.contains("http://localhost:8080/callback"));
    }

    #[test]
    fn test_upstox_order_type() {
        assert_eq!(UpstoxBroker::upstox_order_type(OrderType::Market), "MARKET");
        assert_eq!(UpstoxBroker::upstox_order_type(OrderType::Limit), "LIMIT");
        assert_eq!(UpstoxBroker::upstox_order_type(OrderType::StopLoss), "SL");
        assert_eq!(
            UpstoxBroker::upstox_order_type(OrderType::StopLossLimit),
            "SL-M"
        );
    }

    #[test]
    fn test_upstox_transaction_type() {
        assert_eq!(
            UpstoxBroker::upstox_transaction_type(TradeDirection::Long),
            "BUY"
        );
        assert_eq!(
            UpstoxBroker::upstox_transaction_type(TradeDirection::Short),
            "SELL"
        );
    }

    #[test]
    fn test_parse_order_status() {
        assert_eq!(
            UpstoxBroker::parse_order_status("COMPLETE", 10, 10),
            OrderStatus::Filled
        );
        assert_eq!(
            UpstoxBroker::parse_order_status("PENDING", 0, 10),
            OrderStatus::Pending
        );
        assert_eq!(
            UpstoxBroker::parse_order_status("REJECTED", 0, 10),
            OrderStatus::Rejected {
                reason: "Order rejected by Upstox".into()
            }
        );
        assert_eq!(
            UpstoxBroker::parse_order_status("CANCELLED", 0, 10),
            OrderStatus::Cancelled
        );
    }
}
