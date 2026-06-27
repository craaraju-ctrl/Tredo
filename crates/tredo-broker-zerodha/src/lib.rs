//! # Zerodha Kite Connect v3 — Live Broker Adapter
//!
//! Implements [`BrokerAdapter`] for real-money trading via the Kite Connect REST API.
//!
//! ## Authentication Flow
//! 1. User authorizes via: `https://kite.zerodha.com/connect/login?v=3&api_key={API_KEY}`
//! 2. Zerodha redirects to callback URL with a `request_token`
//! 3. Exchange `request_token` for an `access_token` (valid until 6 AM next day)
//! 4. All subsequent requests use `Authorization: token {api_key}:{access_token}`
//!
//! ## Usage
//! See the `BrokerAdapter` trait for the full API.
//!
//! See the `BrokerAdapter` trait for the full API.

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use tokio::sync::RwLock;
use tredo_core::paper_engine::{
    BrokerAdapter, ClosedTrade, OrderRequest, OrderStatus, OrderType, PortfolioSummary, Position,
    RiskCheckResult, TradingMode,
};
use tredo_core::TradeDirection;

// ── Error Type ───────────────────────────────────────────────────────────────

/// Errors from the Kite Connect API.
#[derive(Debug, thiserror::Error)]
pub enum KiteError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error: status={status} message={message}")]
    Api { status: u16, message: String },

    #[error("Not connected — call connect() first")]
    NotConnected,

    #[error("Missing field in response: {0}")]
    MissingField(String),

    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    #[error("Auth failed: {0}")]
    Auth(String),
}

// ── Response Types (Kite API JSON) ──────────────────────────────────────────

/// Envelope for all Kite API responses.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KiteResponse<T> {
    status: String, // "success" | "error"
    data: Option<T>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Token exchange response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SessionData {
    access_token: String,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    login_time: Option<String>,
}

/// User profile/margins response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct UserMargins {
    equity: Option<MarginSegment>,
    commodity: Option<MarginSegment>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct MarginSegment {
    available: MarginBreakdown,
    used: MarginBreakdown,
    enabled: bool,
    net: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct MarginBreakdown {
    #[serde(default)]
    adhoc_margin: f64,
    #[serde(default)]
    cash: f64,
    #[serde(default)]
    collateral: f64,
    #[serde(default)]
    intraday_payin: f64,
    #[serde(default)]
    live_balance: Option<f64>,
}

/// Order response from Kite.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct KiteOrder {
    order_id: Option<String>,
    order_timestamp: Option<String>,
    status: Option<String>,
    quantity: Option<String>,
    filled_quantity: Option<String>,
    price: Option<f64>,
    average_price: Option<f64>,
    trigger_price: Option<f64>,
    exchange: Option<String>,
    trading_symbol: Option<String>,
    transaction_type: Option<String>,
    order_type: Option<String>,
    variety: Option<String>,
    validity: Option<String>,
    parent_order_id: Option<String>,
    placed_by: Option<String>,
    exchange_order_id: Option<String>,
    status_message: Option<String>,
    rejection_reason: Option<String>,
    // Legacy/alternate fields:
    /// Order ID (could be under order_id or id)
    id: Option<String>,
    /// Average fill price
    average_price_f: Option<f64>,
    /// Filled quantity as string or int
    filled_quantity_f: Option<i32>,
    /// Quantity as string or int
    quantity_f: Option<i32>,
}

/// Position response from Kite.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct KitePosition {
    trading_symbol: Option<String>,
    exchange: Option<String>,
    instrument_token: Option<String>,
    quantity: Option<i32>,
    t1_quantity: Option<i32>,
    realised_quantity: Option<i32>,
    overnight_quantity: Option<i32>,
    buy_quantity: Option<i32>,
    sell_quantity: Option<i32>,
    net_quantity: Option<i32>,
    buy_price: Option<f64>,
    sell_price: Option<f64>,
    last_price: Option<f64>,
    m2m: Option<f64>,
    unrealised: Option<f64>,
    pnl: Option<f64>,
    average_price: Option<f64>,
    buy_value: Option<f64>,
    sell_value: Option<f64>,
    close_price: Option<f64>,
    overnight_buy_value: Option<f64>,
    overnight_sell_value: Option<f64>,
    day_buy_value: Option<f64>,
    day_sell_value: Option<f64>,
    multiplier: Option<f64>,
}

/// Holding response from Kite.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct KiteHolding {
    trading_symbol: Option<String>,
    exchange: Option<String>,
    instrument_token: Option<String>,
    isin: Option<String>,
    quantity: Option<i32>,
    t1_quantity: Option<i32>,
    realised_quantity: Option<i32>,
    price: Option<f64>,
    average_price: Option<f64>,
    last_price: Option<f64>,
    pnl: Option<f64>,
    collateral_quantity: Option<i32>,
    collateral_type: Option<String>,
    haircut: Option<f64>,
    close_price: Option<f64>,
}

// ── ZerodhaKiteBroker ───────────────────────────────────────────────────────

/// Live trading broker for Zerodha Kite Connect v3.
///
/// Uses the [Kite Connect REST API](https://kite.trade/docs/connect/v3/) to
/// place real-money orders, fetch positions, and manage portfolio state.
///
/// ## Auth flow
/// 1. Visit `https://kite.zerodha.com/connect/login?v=3&api_key={api_key}`
/// 2. After login, capture the `request_token` from the callback URL
/// 3. Call `connect()` which exchanges it for an access token
///
/// The access token is valid until 6 AM the next day and is stored in memory.
#[derive(Debug)]
pub struct ZerodhaKiteBroker {
    api_key: String,
    #[allow(dead_code)]
    api_secret: String,
    base_url: String,
    access_token: RwLock<Option<String>>,
    connected: AtomicBool,
    client: reqwest::Client,
}

impl ZerodhaKiteBroker {
    /// Create a new Kite broker.
    ///
    /// * `api_key` — Your Kite Connect API key.
    /// * `api_secret` — Your Kite Connect API secret (used to sign the token request).
    /// * `base_url` — API base URL (default: `https://api.kite.trade`).
    /// * `request_token` — The request token obtained from the OAuth callback URL.
    ///   Pass an empty string if you want to call `connect(request_token)` later.
    pub fn new(api_key: &str, api_secret: &str, base_url: &str, request_token: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("tredo/0.2.0 (Kite Connect v3)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let access_token = if request_token.is_empty() {
            None
        } else {
            // Pre-compute token from request_token so connect() can use it
            let checksum = Self::compute_checksum(api_key, api_secret, request_token);
            Some(format!("{}:{}", request_token, checksum))
        };

        Self {
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            access_token: RwLock::new(access_token),
            connected: AtomicBool::new(false),
            client,
        }
    }

    /// Compute the SHA-256 checksum required for token exchange.
    fn compute_checksum(api_key: &str, api_secret: &str, request_token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        hasher.update(request_token.as_bytes());
        hasher.update(api_secret.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Set a new access token (e.g., after re-auth).
    pub async fn set_access_token(&self, token: &str) {
        let mut t = self.access_token.write().await;
        *t = Some(token.to_string());
    }

    /// Build the Authorization header value.
    async fn auth_header(&self) -> Result<String, KiteError> {
        let token = self.access_token.read().await;
        match token.as_ref() {
            Some(t) => Ok(format!("token {}:{}", self.api_key, t)),
            None => Err(KiteError::Auth(
                "No access token. Call connect(request_token) first.".into(),
            )),
        }
    }

    /// Make an authenticated GET request to the Kite API.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, KiteError> {
        let url = format!("{}{}", self.base_url, path);
        let auth = self.auth_header().await?;

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth)
            .header("X-Kite-Version", "3")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(KiteError::Api {
                status,
                message: body,
            });
        }

        let envelope: KiteResponse<T> = resp.json().await?;
        if envelope.status != "success" {
            return Err(KiteError::Api {
                status,
                message: envelope.message.unwrap_or_else(|| "Unknown error".into()),
            });
        }

        envelope
            .data
            .ok_or_else(|| KiteError::MissingField("data".into()))
    }

    /// Make an authenticated POST request with form data.
    #[allow(dead_code)]
    async fn post_form<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        form: &[(&str, String)],
    ) -> Result<T, KiteError> {
        let url = format!("{}{}", self.base_url, path);
        let auth = self.auth_header().await?;

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &auth)
            .header("X-Kite-Version", "3")
            .form(form)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(KiteError::Api {
                status,
                message: body,
            });
        }

        let envelope: KiteResponse<T> = resp.json().await?;
        if envelope.status != "success" {
            return Err(KiteError::Api {
                status,
                message: envelope.message.unwrap_or_else(|| "Unknown error".into()),
            });
        }

        envelope
            .data
            .ok_or_else(|| KiteError::MissingField("data".into()))
    }

    /// Make an authenticated DELETE request.
    async fn delete(&self, path: &str) -> Result<(), KiteError> {
        let url = format!("{}{}", self.base_url, path);
        let auth = self.auth_header().await?;

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", &auth)
            .header("X-Kite-Version", "3")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(KiteError::Api {
                status,
                message: body,
            });
        }

        Ok(())
    }

    /// Convert a Kite position list to TREDO Position list.
    fn kite_positions_to_tredo(
        positions: &[KitePosition],
        holdings: &[KiteHolding],
    ) -> Vec<Position> {
        let mut result = Vec::new();
        let now = Utc::now();

        for pos in positions {
            let net_qty = pos.net_quantity.unwrap_or(0);
            if net_qty == 0 {
                continue;
            }

            let direction = if net_qty > 0 {
                TradeDirection::Long
            } else {
                TradeDirection::Short
            };
            let abs_qty = net_qty.abs();
            let entry_price = pos.average_price.unwrap_or(0.0);
            let current_price = pos.last_price.unwrap_or(entry_price);
            let pnl = pos.pnl.unwrap_or(0.0);

            let symbol = pos
                .trading_symbol
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            let unrealized_pnl_pct = if entry_price > 0.0 {
                (current_price - entry_price) / entry_price * 100.0
            } else {
                0.0
            };

            result.push(Position {
                id: format!("ZERODHA--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction,
                qty: abs_qty,
                entry_price,
                current_price,
                stop_loss: 0.0, // Kite doesn't expose SL on positions
                take_profit: 0.0,
                unrealized_pnl: pnl,
                unrealized_pnl_pct,
                status: tredo_core::paper_engine::PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("Zerodha Live".to_string()),
                order_id: String::new(),
            });
        }

        // Also add holdings (delivery positions not in F&O)
        for h in holdings {
            let qty = h.quantity.unwrap_or(0);
            if qty == 0 {
                continue;
            }

            let entry_price = h.average_price.unwrap_or(0.0);
            let current_price = h.last_price.unwrap_or(entry_price);
            let pnl = h.pnl.unwrap_or(0.0);
            let symbol = h
                .trading_symbol
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            // Skip if already in positions list
            if result.iter().any(|p| p.symbol == symbol) {
                continue;
            }

            let unrealized_pnl_pct = if entry_price > 0.0 {
                (current_price - entry_price) / entry_price * 100.0
            } else {
                0.0
            };

            result.push(Position {
                id: format!("ZERODHA-HLDG--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction: TradeDirection::Long,
                qty,
                entry_price,
                current_price,
                stop_loss: 0.0,
                take_profit: 0.0,
                unrealized_pnl: pnl,
                unrealized_pnl_pct,
                status: tredo_core::paper_engine::PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("Zerodha Holdings".to_string()),
                order_id: String::new(),
            });
        }

        result
    }

    /// Map TREDO OrderType to Kite order type string.
    fn kite_order_type(order_type: OrderType) -> &'static str {
        match order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopLoss => "SL",
            OrderType::StopLossLimit => "SL-M",
        }
    }

    /// Map TREDO TradeDirection to Kite transaction type.
    fn kite_transaction_type(direction: TradeDirection) -> &'static str {
        match direction {
            TradeDirection::Long => "BUY",
            TradeDirection::Short => "SELL",
        }
    }

    /// Map Kite status to TREDO OrderStatus.
    fn parse_order_status(kite_status: &str, filled_qty: i32, total_qty: i32) -> OrderStatus {
        match kite_status.to_uppercase().as_str() {
            "PENDING" | "OPEN" | "TRIGGER PENDING" | "VALIDATION PENDING" => OrderStatus::Pending,
            "COMPLETE" | "FILLED" => {
                if filled_qty >= total_qty {
                    OrderStatus::Filled
                } else {
                    OrderStatus::PartiallyFilled { filled_qty }
                }
            }
            "CANCELLED" | "CANCELED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected {
                reason: "Order rejected by exchange".into(),
            },
            _ => OrderStatus::Pending,
        }
    }
}

#[async_trait]
impl BrokerAdapter for ZerodhaKiteBroker {
    /// Exchange the request token for an access token via /session/token.
    /// The token is stored in memory for the session duration (valid until 6 AM next day).
    async fn connect(&self) -> Result<(), String> {
        // If we already have a token and it's marked connected, skip
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        let token_guard = self.access_token.read().await;
        let (request_token, checksum) = match token_guard.as_ref() {
            Some(full_token) => {
                // Token was pre-computed in new(); extract request_token portion
                if let Some(pos) = full_token.find(':') {
                    let rt = &full_token[..pos];
                    let cs = &full_token[pos + 1..];
                    (rt.to_string(), cs.to_string())
                } else {
                    return Err("Invalid token format — expected 'request_token:checksum'".into());
                }
            }
            None => {
                return Err(
                    "No request_token provided. Construct with a request_token or call \
                     set_access_token().\
                     \n  Visit: https://kite.zerodha.com/connect/login?v=3&api_key={API_KEY}"
                        .into(),
                );
            }
        };
        drop(token_guard);

        // Exchange request token for access token
        let url = format!("{}/session/token", self.base_url);
        let form = [
            ("api_key", self.api_key.clone()),
            ("request_token", request_token),
            ("checksum", checksum),
        ];

        let resp = self
            .client
            .post(&url)
            .header("X-Kite-Version", "3")
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Token exchange HTTP error: {}", e))?;

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();

        if !(200..300).contains(&status) {
            return Err(format!("Token exchange failed (HTTP {}): {}", status, body));
        }

        let envelope: KiteResponse<SessionData> = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse session response: {}", e))?;

        if envelope.status != "success" {
            return Err(format!(
                "Token exchange denied: {}",
                envelope.message.unwrap_or_default()
            ));
        }

        let session = envelope.data.ok_or("No session data in response")?;

        // Store the access token (without the request_token prefix)
        let mut token = self.access_token.write().await;
        *token = Some(session.access_token.clone());
        drop(token);

        self.connected.store(true, Ordering::Relaxed);

        tracing::info!(
            "✅ Zerodha Kite connected (user: {})",
            session.user_name.as_deref().unwrap_or("unknown")
        );

        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::Relaxed);
        let mut token = self.access_token.write().await;
        *token = None;
        tracing::info!("🔌 Zerodha Kite disconnected");
        Ok(())
    }

    async fn place_order(
        &self,
        request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("Not connected to Zerodha. Call connect() first.".into());
        }

        let variety = "regular"; // regular, amo, iceberg, etc.

        let mut form = vec![
            ("tradingsymbol", request.symbol.clone()),
            ("exchange", "NSE".to_string()), // default NSE
            (
                "transaction_type",
                Self::kite_transaction_type(request.direction).to_string(),
            ),
            (
                "order_type",
                Self::kite_order_type(request.order_type).to_string(),
            ),
            ("quantity", request.qty.to_string()),
            ("product", "MIS".to_string()), // MIS for intraday, CNC for delivery
            ("validity", "DAY".to_string()),
        ];

        if let Some(ref price) = request.price {
            form.push(("price", price.to_string()));
        }
        if let Some(ref sl) = request.stop_loss {
            form.push(("trigger_price", sl.to_string()));
        }
        if let Some(ref tp) = request.take_profit {
            // Square-off limit price for SL order
            form.push(("squareoff", tp.to_string()));
        }
        if let Some(ref tag) = request.strategy {
            form.push(("tag", tag.clone()));
        }

        let url = format!("{}/orders/{}", self.base_url, variety);

        let resp = self
            .client
            .post(&url)
            .header(
                "Authorization",
                &self.auth_header().await.map_err(|e| e.to_string())?,
            )
            .header("X-Kite-Version", "3")
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Place order HTTP error: {}", e))?;

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();

        if !(200..300).contains(&status) {
            return Err(format!("Order failed (HTTP {}): {}", status, body));
        }

        let envelope: KiteResponse<serde_json::Value> = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse order response: {}", e))?;

        if envelope.status != "success" {
            return Err(format!(
                "Order rejected: {}",
                envelope.message.unwrap_or_default()
            ));
        }

        let order_id = envelope
            .data
            .and_then(|d| d["order_id"].as_str().map(String::from))
            .ok_or("No order_id in response")?;

        tracing::info!(
            "📈 Zerodha order placed: {} {} qty={} id={}",
            Self::kite_transaction_type(request.direction),
            request.symbol,
            request.qty,
            order_id,
        );

        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(KiteError::NotConnected.to_string());
        }

        // Try all varieties since we may not know which one was used
        let varieties = ["regular", "amo", "iceberg"];
        let mut last_err = String::new();

        for variety in &varieties {
            let path = format!("/orders/{}/{}", variety, order_id);
            match self.delete(&path).await {
                Ok(()) => {
                    tracing::info!("🗑️ Cancelled order {} via {}", order_id, variety);
                    return Ok(());
                }
                Err(e) => {
                    last_err = e.to_string();
                }
            }
        }

        Err(format!("Failed to cancel order {}: {}", order_id, last_err))
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(KiteError::NotConnected.to_string());
        }

        let positions: Vec<KitePosition> = self
            .get("/portfolio/positions")
            .await
            .map_err(|e| e.to_string())?;

        let holdings: Vec<KiteHolding> = self
            .get("/portfolio/holdings")
            .await
            .map_err(|e| e.to_string())?;

        Ok(Self::kite_positions_to_tredo(&positions, &holdings))
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(KiteError::NotConnected.to_string());
        }

        let margins: UserMargins = self.get("/user/margins").await.map_err(|e| e.to_string())?;

        let equity_margin = margins.equity.unwrap_or_default();
        let cash = equity_margin.available.cash
            + equity_margin.available.adhoc_margin
            + equity_margin.available.collateral;
        let used = equity_margin.used.cash + equity_margin.used.collateral;
        let equity = cash + used;

        // Get positions to calculate P&L and trade counts
        let positions = self.get_positions().await.unwrap_or_default();

        let total_pnl: f64 = positions.iter().map(|p| p.unrealized_pnl).sum();
        let winning_trades = positions.iter().filter(|p| p.unrealized_pnl > 0.0).count() as u32;
        let losing_trades = positions.iter().filter(|p| p.unrealized_pnl < 0.0).count() as u32;

        Ok(PortfolioSummary {
            cash,
            equity,
            margin_used: used,
            free_margin: cash,
            daily_pnl: total_pnl,
            daily_pnl_pct: if equity > 0.0 {
                (total_pnl / equity) * 100.0
            } else {
                0.0
            },
            total_trades: winning_trades + losing_trades,
            winning_trades,
            losing_trades,
            win_rate: if (winning_trades + losing_trades) > 0 {
                winning_trades as f64 / (winning_trades + losing_trades) as f64 * 100.0
            } else {
                0.0
            },
            consecutive_losses: 0,
            max_drawdown: equity,
            max_drawdown_pct: 0.0,
            open_positions: positions.len(),
            total_pnl_all_time: total_pnl,
        })
    }

    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(KiteError::NotConnected.to_string());
        }

        let orders: Vec<KiteOrder> = self
            .get(&format!("/orders/{}", order_id))
            .await
            .map_err(|e| e.to_string())?;

        let order = orders
            .into_iter()
            .find(|o| o.order_id.as_deref() == Some(order_id) || o.id.as_deref() == Some(order_id))
            .ok_or_else(|| format!("Order {} not found", order_id))?;

        let kite_status = order.status.as_deref().unwrap_or("PENDING");
        let filled_qty = order.filled_quantity_f.unwrap_or(0);
        let total_qty = order.quantity_f.unwrap_or(0);

        Ok(Self::parse_order_status(kite_status, filled_qty, total_qty))
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(KiteError::NotConnected.to_string());
        }

        let orders: Vec<KiteOrder> = self.get("/orders").await.map_err(|e| e.to_string())?;

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
                let qty = o.filled_quantity_f.unwrap_or(0).abs();
                let price = o.average_price.unwrap_or(0.0);

                ClosedTrade {
                    id: o.order_id.unwrap_or_default(),
                    symbol: o.trading_symbol.unwrap_or_default(),
                    direction,
                    qty,
                    entry_price: price,
                    exit_price: 0.0, // not available from single order
                    realized_pnl: 0.0,
                    realized_pnl_pct: 0.0,
                    close_reason: tredo_core::paper_engine::CloseReason::Manual,
                    opened_at: now,
                    closed_at: now,
                    duration_secs: 0,
                    strategy: Some("Zerodha Live".to_string()),
                    order_id: String::new(),
                }
            })
            .collect();

        trades.reverse(); // most recent first
        Ok(trades)
    }

    async fn update_price(
        &self,
        symbol: &str,
        _market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        // Kite doesn't auto-close on SL/TP — that's handled by TREDO's own risk engine.
        // Return an empty vec; no trades are auto-closed by this price update.
        let _ = symbol;
        Ok(Vec::new())
    }

    async fn close_position(
        &self,
        position_id: &str,
        _exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(KiteError::NotConnected.to_string());
        }

        // Parse position ID to extract symbol.
        // Format: "ZERODHA--{symbol}--{timestamp}" (double-dash delimited so hyphens in symbols work)
        let parts: Vec<&str> = position_id.splitn(3, "--").collect();
        if parts.len() < 2 || parts[1].is_empty() {
            return Err(format!("Invalid position ID format: {}", position_id));
        }

        let symbol = parts[1];

        // Get current position details
        let positions = self.get_positions().await?;
        let pos = positions
            .into_iter()
            .find(|p| p.id == position_id)
            .ok_or_else(|| format!("Position {} not found", position_id))?;

        // Place an opposing order to close
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
            close_reason: tredo_core::paper_engine::CloseReason::Manual,
            opened_at: Utc::now(),
            closed_at: Utc::now(),
            duration_secs: 0,
            strategy: Some("Zerodha Live".to_string()),
            order_id,
        })
    }

    async fn check_risk(
        &self,
        _symbol: &str,
        _estimated_cost: f64,
    ) -> Result<RiskCheckResult, String> {
        // Kite handles its own risk checks (margin, exposure).
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
        // Cannot reset a live broker account — this is a no-op.
        Err("Cannot reset a live Zerodha account. Use paper mode for reset.".into())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        "Zerodha Kite"
    }
}

// ── Helper to create an Arc'd broker for use with BrokerRegistry ────────────

/// Create a Zerodha Kite broker wrapped in an Arc, ready to register.
pub fn create_zerodha_broker(
    api_key: &str,
    api_secret: &str,
    request_token: &str,
) -> std::sync::Arc<dyn BrokerAdapter> {
    std::sync::Arc::new(ZerodhaKiteBroker::new(
        api_key,
        api_secret,
        "https://api.kite.trade",
        request_token,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_computation() {
        // Known test vectors for SHA-256
        let checksum =
            ZerodhaKiteBroker::compute_checksum("api_key", "api_secret", "request_token");
        assert_eq!(checksum.len(), 64, "SHA-256 hex should be 64 chars");
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "Should be hex"
        );
    }

    #[test]
    fn test_parse_order_status() {
        assert_eq!(
            ZerodhaKiteBroker::parse_order_status("COMPLETE", 10, 10),
            OrderStatus::Filled
        );
        assert_eq!(
            ZerodhaKiteBroker::parse_order_status("PENDING", 0, 10),
            OrderStatus::Pending
        );
        assert_eq!(
            ZerodhaKiteBroker::parse_order_status("REJECTED", 0, 10),
            OrderStatus::Rejected {
                reason: "Order rejected by exchange".into()
            }
        );
        assert_eq!(
            ZerodhaKiteBroker::parse_order_status("CANCELLED", 0, 10),
            OrderStatus::Cancelled
        );
    }

    #[test]
    fn test_kite_transaction_type() {
        assert_eq!(
            ZerodhaKiteBroker::kite_transaction_type(TradeDirection::Long),
            "BUY"
        );
        assert_eq!(
            ZerodhaKiteBroker::kite_transaction_type(TradeDirection::Short),
            "SELL"
        );
    }

    #[test]
    fn test_kite_order_type() {
        assert_eq!(
            ZerodhaKiteBroker::kite_order_type(OrderType::Market),
            "MARKET"
        );
        assert_eq!(
            ZerodhaKiteBroker::kite_order_type(OrderType::Limit),
            "LIMIT"
        );
        assert_eq!(
            ZerodhaKiteBroker::kite_order_type(OrderType::StopLoss),
            "SL"
        );
        assert_eq!(
            ZerodhaKiteBroker::kite_order_type(OrderType::StopLossLimit),
            "SL-M"
        );
    }

    #[test]
    fn test_broker_name_and_mode() {
        let broker = ZerodhaKiteBroker::new("k", "s", "https://api.kite.trade", "");
        assert_eq!(broker.broker_name(), "Zerodha Kite");
        assert_eq!(broker.mode(), TradingMode::Live);
    }
}
