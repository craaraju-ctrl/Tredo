//! # Angel One SmartAPI — Free Indian Discount Broker Adapter
//!
//! Implements [`BrokerAdapter`] for trading via the Angel One SmartAPI REST API.
//! Angel One (formerly Angel Broking) is a popular free Indian discount broker.
//!
//! ## Authentication Flow
//! 1. Register app at https://smartapi.angelbroking.com/ to get `api_key`
//! 2. Generate TOTP and login with `client_id` + `pin` + `totp`
//! 3. Receive JWT `auth_token` and `refresh_token`
//! 4. All subsequent requests use `Authorization: Bearer {auth_token}`
//!    plus `X-PrivateKey: {api_key}` and `X-ClientCode: {client_id}`
//!
//! ## Environment Variables
//! - `ANGEL_API_KEY` — Your SmartAPI key
//! - `ANGEL_CLIENT_ID` — Your client code (trading account ID)
//! - `ANGEL_PIN` — Your trading PIN
//! - `ANGEL_TOTP_SECRET` — Your TOTP secret for 2FA
//! - `ANGEL_AUTH_TOKEN` — Pre-obtained auth token (skip login flow)

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tredo_core::paper_engine::{
    BrokerAdapter, CloseReason, ClosedTrade, OrderRequest, OrderStatus, OrderType,
    PortfolioSummary, Position, PositionStatus, RiskCheckResult, TradingMode,
};
use tredo_core::TradeDirection;

// ── Error Type ───────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AngelOneError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error: status={status} message={message}")]
    Api { status: u16, message: String },

    #[error("Not connected — call connect() first")]
    NotConnected,

    #[error("Missing field: {0}")]
    MissingField(String),

    #[error("Auth failed: {0}")]
    Auth(String),

    #[error("Token expired")]
    TokenExpired,
}

// ── Response Types ───────────────────────────────────────────────────────────

/// Angel One API envelope response.
#[derive(Debug, Deserialize)]
struct AngelResponse<T> {
    status: bool,
    data: Option<T>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    errorcode: Option<String>,
}

/// Login response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct LoginResponse {
    #[serde(default)]
    auth_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    login_time: Option<String>,
    #[serde(default)]
    feed_token: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

/// Login request
#[derive(Debug, Serialize)]
struct LoginRequest {
    clientcode: String,
    password: String,
    totp: String,
    state: String,
}

/// Order placement request
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaceOrderRequest {
    variety: String,
    tradingsymbol: String,
    symboltoken: String,
    exchange: String,
    transaction_type: String,
    order_type: String,
    quantity: String,
    price: String,
    trigger_price: String,
    squareoff: String,
    stoploss: String,
    producttype: String,
    validity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

/// Order response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct OrderResponse {
    #[serde(default)]
    orderid: Option<String>,
    #[serde(default)]
    uniqueorderid: Option<String>,
}

/// Position response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AngelPosition {
    #[serde(default)]
    tradingsymbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    symboltoken: Option<String>,
    #[serde(default)]
    quantity: Option<i32>,
    #[serde(default)]
    buyquantity: Option<i32>,
    #[serde(default)]
    sellquantity: Option<i32>,
    #[serde(default)]
    netquantity: Option<i32>,
    #[serde(default)]
    buyavgprice: Option<f64>,
    #[serde(default)]
    sellavgprice: Option<f64>,
    #[serde(default)]
    ltp: Option<f64>,
    #[serde(default)]
    mtomprofitandloss: Option<f64>,
    #[serde(default)]
    pnl: Option<f64>,
    #[serde(default)]
    producttype: Option<String>,
    #[serde(default)]
    multiplier: Option<f64>,
}

/// Holding response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AngelHolding {
    #[serde(default)]
    tradingsymbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    symboltoken: Option<String>,
    #[serde(default)]
    quantity: Option<i32>,
    #[serde(default)]
    averageprice: Option<f64>,
    #[serde(default)]
    ltp: Option<f64>,
    #[serde(default)]
    profitandloss: Option<f64>,
    #[serde(default)]
    haircut: Option<f64>,
}

/// Order/filled trade response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AngelOrder {
    #[serde(default)]
    orderid: Option<String>,
    #[serde(default)]
    tradingsymbol: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    transaction_type: Option<String>,
    #[serde(default)]
    order_type: Option<String>,
    #[serde(default)]
    quantity: Option<i32>,
    #[serde(default)]
    filledqty: Option<i32>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    averageprice: Option<f64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    order_status: Option<String>,
    #[serde(default)]
    updatetime: Option<String>,
    #[serde(default)]
    reject_reason: Option<String>,
}

/// Funds/Margin response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AngelFunds {
    #[serde(default)]
    totalavailablemargin: Option<f64>,
    #[serde(default)]
    netmargin: Option<f64>,
    #[serde(default)]
    availablecash: Option<f64>,
    #[serde(default)]
    payin_amount: Option<f64>,
    #[serde(default)]
    adhoc_margin: Option<f64>,
    #[serde(default)]
    collateral: Option<f64>,
    #[serde(default)]
    utilised_margin: Option<f64>,
}

// ── AngelOneBroker ────────────────────────────────────────────────────────────

/// Live trading broker for Angel One SmartAPI.
pub struct AngelOneBroker {
    api_key: String,
    client_id: String,
    pin: String,
    totp_secret: Option<String>,
    base_url: String,
    auth_token: RwLock<Option<String>>,
    feed_token: RwLock<Option<String>>,
    connected: AtomicBool,
    client: reqwest::Client,
}

impl std::fmt::Debug for AngelOneBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AngelOneBroker")
            .field("base_url", &self.base_url)
            .field("connected", &self.connected)
            .finish()
    }
}

impl AngelOneBroker {
    /// Create a new Angel One broker.
    ///
    /// * `api_key` — Your SmartAPI key
    /// * `client_id` — Your client code
    /// * `pin` — Your trading PIN
    /// * `totp_secret` — TOTP secret for 2FA
    /// * `auth_token` — Pre-obtained auth token (or empty)
    pub fn new(
        api_key: &str,
        client_id: &str,
        pin: &str,
        totp_secret: Option<String>,
        auth_token: &str,
    ) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("tredo/0.2.0 (Angel One SmartAPI)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key: api_key.to_string(),
            client_id: client_id.to_string(),
            pin: pin.to_string(),
            totp_secret,
            base_url: "https://apiconnect.angelbroking.com".to_string(),
            auth_token: RwLock::new(if auth_token.is_empty() {
                None
            } else {
                Some(auth_token.to_string())
            }),
            feed_token: RwLock::new(None),
            connected: AtomicBool::new(false),
            client,
        }
    }

    /// Generate a TOTP code from the secret (simple RFC 6238 implementation).
    fn generate_totp(secret: &str) -> Result<String, String> {
        let time_step = chrono::Utc::now().timestamp() / 30;
        let msg = time_step.to_be_bytes();
        use base64::Engine;
        let key = base64::engine::general_purpose::STANDARD
            .decode(secret)
            .map_err(|_| "Invalid base64 TOTP secret".to_string())?;
        use hmac::{Hmac, Mac};
        use sha1::Sha1;
        type HmacSha1 = Hmac<Sha1>;

        let mut mac = HmacSha1::new_from_slice(&key)
            .map_err(|_| "Invalid key length for TOTP".to_string())?;
        mac.update(&msg);
        let result = mac.finalize().into_bytes();
        let offset = (result[result.len() - 1] & 0xf) as usize;
        let code = ((result[offset] & 0x7f) as u32) << 24
            | (result[offset + 1] as u32) << 16
            | (result[offset + 2] as u32) << 8
            | (result[offset + 3] as u32);
        let totp = code % 1_000_000;
        Ok(format!("{:06}", totp))
    }

    /// Compute SHA-256 hash of the PIN for login.
    fn hash_pin(pin: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(pin.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Build common headers for Angel One API requests.
    fn headers(&self, token: Option<&str>) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(&self.api_key) {
            headers.insert(HeaderName::from_static("x-privatekey"), v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.client_id) {
            headers.insert(HeaderName::from_static("x-clientcode"), v);
        }
        if let Ok(v) = HeaderValue::from_str("application/json") {
            headers.insert(HeaderName::from_static("accept"), v.clone());
            headers.insert(HeaderName::from_static("content-type"), v);
        }
        if let Some(token) = token {
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", token)) {
                headers.insert(HeaderName::from_static("authorization"), v);
            }
        }
        headers
    }

    /// Make an authenticated POST request.
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
        requires_auth: bool,
    ) -> Result<T, AngelOneError> {
        let url = format!("{}{}", self.base_url, path);
        let token = if requires_auth {
            Some(
                self.auth_token
                    .read()
                    .await
                    .clone()
                    .ok_or(AngelOneError::Auth("Not authenticated".into()))?,
            )
        } else {
            None
        };
        let headers = if let Some(ref t) = token {
            self.headers(Some(t))
        } else {
            self.headers(None)
        };

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 401 {
            return Err(AngelOneError::TokenExpired);
        }
        if !(200..300).contains(&status) {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AngelOneError::Api {
                status,
                message: body_text,
            });
        }

        let envelope: AngelResponse<T> = resp.json().await?;
        if !envelope.status {
            return Err(AngelOneError::Api {
                status,
                message: envelope.message.unwrap_or_else(|| "Unknown error".into()),
            });
        }

        envelope
            .data
            .ok_or_else(|| AngelOneError::MissingField("data".into()))
    }

    /// Make a GET request.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, AngelOneError> {
        let url = format!("{}{}", self.base_url, path);
        let token = self
            .auth_token
            .read()
            .await
            .clone()
            .ok_or(AngelOneError::Auth("Not authenticated".into()))?;
        let headers = self.headers(Some(&token));

        let resp = self.client.get(&url).headers(headers).send().await?;

        let status = resp.status().as_u16();
        if status == 401 {
            return Err(AngelOneError::TokenExpired);
        }
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(AngelOneError::Api {
                status,
                message: body,
            });
        }

        let envelope: AngelResponse<T> = resp.json().await?;
        if !envelope.status {
            return Err(AngelOneError::Api {
                status,
                message: envelope.message.unwrap_or_else(|| "Unknown error".into()),
            });
        }

        envelope
            .data
            .ok_or_else(|| AngelOneError::MissingField("data".into()))
    }

    /// Map TREDO OrderType to Angel One type.
    fn angel_order_type(order_type: OrderType) -> &'static str {
        match order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopLoss => "STOPLOSS",
            OrderType::StopLossLimit => "STOPLIMIT",
        }
    }

    /// Map TREDO direction to Angel One type.
    fn angel_transaction_type(direction: TradeDirection) -> &'static str {
        match direction {
            TradeDirection::Long => "BUY",
            TradeDirection::Short => "SELL",
        }
    }

    /// Parse Angel One order status.
    fn parse_order_status(status: &str) -> OrderStatus {
        match status.to_uppercase().as_str() {
            "OPEN" | "PENDING" | "TRIGGER_PENDING" => OrderStatus::Pending,
            "COMPLETE" | "FILLED" => OrderStatus::Filled,
            "CANCELLED" | "CANCELED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected {
                reason: "Order rejected by Angel One".into(),
            },
            _ => OrderStatus::Pending,
        }
    }

    /// Map product type.
    fn product_type(tag: Option<&str>) -> &str {
        match tag {
            Some("MIS") => "INTRADAY",
            _ => "DELIVERY",
        }
    }
}

#[async_trait]
impl BrokerAdapter for AngelOneBroker {
    async fn connect(&self) -> Result<(), String> {
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        // If we have a pre-obtained auth token, validate it
        {
            let token = self.auth_token.read().await;
            if token.is_some() {
                // Quick validation by fetching funds
                let url = format!("{}/rest/secure/angelbroking/user/v1/getRMS", self.base_url);
                let headers = self.headers(Some(token.as_ref().unwrap()));
                let resp = self
                    .client
                    .get(&url)
                    .headers(headers)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await;

                if let Ok(r) = resp {
                    if r.status().is_success() {
                        self.connected.store(true, Ordering::Relaxed);
                        tracing::info!("✅ Angel One connected (existing token)");
                        return Ok(());
                    }
                }
            }
        }

        // Generate TOTP and login
        let totp =
            match &self.totp_secret {
                Some(secret) => Self::generate_totp(secret)?,
                None => return Err(
                    "Angel One: TOTP secret required. Set ANGEL_TOTP_SECRET or ANGEL_AUTH_TOKEN"
                        .into(),
                ),
            };

        let login_req = LoginRequest {
            clientcode: self.client_id.clone(),
            password: Self::hash_pin(&self.pin),
            totp,
            state: "WEB".to_string(),
        };

        let body =
            serde_json::to_value(&login_req).map_err(|e| format!("Serialization error: {}", e))?;

        let login_resp: LoginResponse = self
            .post("/rest/secure/angelbroking/user/v1/login", &body, false)
            .await
            .map_err(|e| format!("Angel One login failed: {}", e))?;

        let auth_token = login_resp
            .auth_token
            .ok_or_else(|| "No auth_token in response".to_string())?;

        let feed_token = login_resp.feed_token.unwrap_or_default();

        {
            let mut t = self.auth_token.write().await;
            *t = Some(auth_token);
        }
        {
            let mut t = self.feed_token.write().await;
            *t = Some(feed_token);
        }

        self.connected.store(true, Ordering::Relaxed);
        tracing::info!("✅ Angel One connected (user: {})", self.client_id);
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::Relaxed);
        let mut t = self.auth_token.write().await;
        *t = None;
        tracing::info!("🔌 Angel One disconnected");
        Ok(())
    }

    async fn place_order(
        &self,
        request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AngelOneError::NotConnected.to_string());
        }

        // For Angel One, we use the symbol as the tradingsymbol
        // and a default token. In production, resolve from master contract.
        let symbol_token = "1"; // Placeholder — resolve from symbol

        let order_req = PlaceOrderRequest {
            variety: "NORMAL".to_string(),
            tradingsymbol: request.symbol.clone(),
            symboltoken: symbol_token.to_string(),
            exchange: "NSE".to_string(),
            transaction_type: Self::angel_transaction_type(request.direction).to_string(),
            order_type: Self::angel_order_type(request.order_type).to_string(),
            quantity: request.qty.to_string(),
            price: match request.order_type {
                OrderType::Market => "0".to_string(),
                OrderType::Limit => request
                    .price
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "0".to_string()),
                OrderType::StopLoss | OrderType::StopLossLimit => request
                    .price
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "0".to_string()),
            },
            trigger_price: request
                .stop_loss
                .map(|s| s.to_string())
                .unwrap_or_else(|| "0".to_string()),
            squareoff: request
                .take_profit
                .map(|t| t.to_string())
                .unwrap_or_else(|| "0".to_string()),
            stoploss: "0".to_string(),
            producttype: Self::product_type(request.strategy.as_deref()).to_string(),
            validity: "DAY".to_string(),
            tag: request.strategy.clone(),
        };

        let body =
            serde_json::to_value(&order_req).map_err(|e| format!("Serialization error: {}", e))?;

        let order_resp: OrderResponse = self
            .post("/rest/secure/angelbroking/order/v1/placeOrder", &body, true)
            .await
            .map_err(|e| format!("Angel One place order failed: {}", e))?;

        let order_id = order_resp
            .orderid
            .or(order_resp.uniqueorderid)
            .ok_or_else(|| "No order_id in response".to_string())?;

        tracing::info!(
            "📈 Angel One order placed: {} {} qty={} id={}",
            Self::angel_transaction_type(request.direction),
            request.symbol,
            request.qty,
            order_id
        );

        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AngelOneError::NotConnected.to_string());
        }

        let body = serde_json::json!({
            "variety": "NORMAL",
            "orderid": order_id,
        });

        let _: serde_json::Value = self
            .post(
                "/rest/secure/angelbroking/order/v1/cancelOrder",
                &body,
                true,
            )
            .await
            .map_err(|e| format!("Failed to cancel order {}: {}", order_id, e))?;

        tracing::info!("🗑️ Cancelled Angel One order {}", order_id);
        Ok(())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AngelOneError::NotConnected.to_string());
        }

        let positions: Vec<AngelPosition> = self
            .get("/rest/secure/angelbroking/order/v1/getPosition")
            .await
            .map_err(|e| e.to_string())?;

        let holdings: Vec<AngelHolding> = self
            .get("/rest/secure/angelbroking/portfolio/v1/getHolding")
            .await
            .map_err(|e| e.to_string())?;

        let now = Utc::now();
        let mut result = Vec::new();

        for pos in &positions {
            let net_qty = pos.netquantity.or(pos.quantity).unwrap_or(0);
            if net_qty == 0 {
                continue;
            }
            let direction = if net_qty > 0 {
                TradeDirection::Long
            } else {
                TradeDirection::Short
            };
            let entry_price = pos.buyavgprice.unwrap_or(0.0);
            let current_price = pos.ltp.unwrap_or(entry_price);
            let pnl = pos.mtomprofitandloss.or(pos.pnl).unwrap_or(0.0);
            let symbol = pos
                .tradingsymbol
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            result.push(Position {
                id: format!("ANGEL--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction,
                qty: net_qty.abs(),
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
                strategy: Some("Angel One Live".to_string()),
                order_id: String::new(),
            });
        }

        // Add holdings
        for h in &holdings {
            let qty = h.quantity.unwrap_or(0);
            if qty == 0
                || result
                    .iter()
                    .any(|p| p.symbol == h.tradingsymbol.as_deref().unwrap_or(""))
            {
                continue;
            }
            let symbol = h
                .tradingsymbol
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());
            result.push(Position {
                id: format!("ANGEL-HLDG--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction: TradeDirection::Long,
                qty,
                entry_price: h.averageprice.unwrap_or(0.0),
                current_price: h.ltp.unwrap_or(0.0),
                stop_loss: 0.0,
                take_profit: 0.0,
                unrealized_pnl: h.profitandloss.unwrap_or(0.0),
                unrealized_pnl_pct: 0.0,
                status: PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("Angel One Holdings".to_string()),
                order_id: String::new(),
            });
        }

        Ok(result)
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AngelOneError::NotConnected.to_string());
        }

        let funds: AngelFunds = self
            .get("/rest/secure/angelbroking/user/v1/getRMS")
            .await
            .map_err(|e| e.to_string())?;

        let cash = funds.availablecash.unwrap_or(0.0)
            + funds.adhoc_margin.unwrap_or(0.0)
            + funds.collateral.unwrap_or(0.0);
        let used = funds.utilised_margin.unwrap_or(0.0);
        let total_equity = funds.totalavailablemargin.unwrap_or(cash + used);

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
            return Err(AngelOneError::NotConnected.to_string());
        }

        let order: AngelOrder = self
            .get(&format!(
                "/rest/secure/angelbroking/order/v1/getOrderStatus?orderid={}",
                order_id
            ))
            .await
            .map_err(|e| e.to_string())?;

        let status = order
            .status
            .or(order.order_status)
            .unwrap_or_else(|| "PENDING".to_string());

        Ok(Self::parse_order_status(&status))
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(AngelOneError::NotConnected.to_string());
        }

        let orders: Vec<AngelOrder> = self
            .get("/rest/secure/angelbroking/order/v1/getTradeBook")
            .await
            .map_err(|e| e.to_string())?;

        let now = Utc::now();
        let mut trades: Vec<ClosedTrade> = orders
            .into_iter()
            .filter(|o| {
                let s = o.status.as_deref().or(o.order_status.as_deref());
                s == Some("COMPLETE") || s == Some("FILLED")
            })
            .take(limit)
            .map(|o| {
                let direction = match o.transaction_type.as_deref() {
                    Some("BUY") => TradeDirection::Long,
                    _ => TradeDirection::Short,
                };
                let qty = o.filledqty.unwrap_or(0).abs();
                let price = o.averageprice.unwrap_or(0.0);

                ClosedTrade {
                    id: o.orderid.unwrap_or_default(),
                    symbol: o.tradingsymbol.unwrap_or_default(),
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
                    strategy: Some("Angel One Live".to_string()),
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
            return Err(AngelOneError::NotConnected.to_string());
        }

        let parts: Vec<&str> = position_id.splitn(3, "--").collect();
        if parts.len() < 2 || parts[1].is_empty() {
            return Err(format!("Invalid position ID: {}", position_id));
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
            strategy: Some("Angel One Live".to_string()),
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
        Err("Cannot reset a live Angel One account. Use paper mode.".into())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        "Angel One"
    }
}

// ── Helper ───────────────────────────────────────────────────────────────────

pub fn create_angelone_broker(
    api_key: &str,
    client_id: &str,
    pin: &str,
    totp_secret: Option<String>,
) -> std::sync::Arc<dyn BrokerAdapter> {
    let auth_token = std::env::var("ANGEL_AUTH_TOKEN").ok().unwrap_or_default();
    std::sync::Arc::new(AngelOneBroker::new(
        api_key,
        client_id,
        pin,
        totp_secret,
        &auth_token,
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broker_name_and_mode() {
        let broker = AngelOneBroker::new("k", "c", "p", None, "");
        assert_eq!(broker.broker_name(), "Angel One");
        assert_eq!(broker.mode(), TradingMode::Live);
    }

    #[test]
    fn test_hash_pin() {
        let hash = AngelOneBroker::hash_pin("1234");
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_angel_order_type() {
        assert_eq!(
            AngelOneBroker::angel_order_type(OrderType::Market),
            "MARKET"
        );
        assert_eq!(AngelOneBroker::angel_order_type(OrderType::Limit), "LIMIT");
        assert_eq!(
            AngelOneBroker::angel_order_type(OrderType::StopLoss),
            "STOPLOSS"
        );
        assert_eq!(
            AngelOneBroker::angel_order_type(OrderType::StopLossLimit),
            "STOPLIMIT"
        );
    }

    #[test]
    fn test_direction_mapping() {
        assert_eq!(
            AngelOneBroker::angel_transaction_type(TradeDirection::Long),
            "BUY"
        );
        assert_eq!(
            AngelOneBroker::angel_transaction_type(TradeDirection::Short),
            "SELL"
        );
    }

    #[test]
    fn test_parse_order_status() {
        assert_eq!(
            AngelOneBroker::parse_order_status("COMPLETE"),
            OrderStatus::Filled
        );
        assert_eq!(
            AngelOneBroker::parse_order_status("PENDING"),
            OrderStatus::Pending
        );
        assert_eq!(
            AngelOneBroker::parse_order_status("REJECTED"),
            OrderStatus::Rejected {
                reason: "Order rejected by Angel One".into()
            }
        );
    }
}
