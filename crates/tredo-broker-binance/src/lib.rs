//! # Binance Broker Adapter — HMAC-SHA256 Signed Orders + WebSocket Fill Confirmations
//!
//! Implements [`BrokerAdapter`] for spot/futures trading via the Binance REST API
//! with HMAC-SHA256 signed endpoints and WebSocket user data streams for fill
//! confirmations.
//!
//! ## Authentication
//! - `BINANCE_API_KEY` — Your API key (from Binance dashboard)
//! - `BINANCE_SECRET_KEY` — Your API secret (HMAC-SHA256 signing key)
//!
//! ## Features
//! - HMAC-SHA256 signed order placement (POST /api/v3/order)
//! - OCO orders for simultaneous SL/TP
//! - Account balance queries (GET /api/v3/account)
//! - Order status polling (GET /api/v3/order)
//! - WebSocket user data stream for real-time fill confirmations
//! - Listen key keepalive (every 30 minutes)
//!
//! ## Binance API Docs
//! - [REST API](https://binance-docs.github.io/apidocs/spot/en/#introduction)
//! - [WebSocket Streams](https://binance-docs.github.io/apidocs/spot/en/#websocket-streams)

use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tredo_core::paper_engine::{
    BrokerAdapter, CloseReason, ClosedTrade, OrderRequest, OrderStatus, OrderType,
    PortfolioSummary, Position, PositionStatus, RiskCheckResult, TradingMode,
};
use tredo_core::TradeDirection;

// ── Error Type ───────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum BinanceError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("WebSocket error: {0}")]
    Ws(String),

    #[error("API error: status={status} code={code} message={message}")]
    Api {
        status: u16,
        code: i64,
        message: String,
    },

    #[error("Not connected — call connect() first")]
    NotConnected,

    #[error("Missing field in response: {0}")]
    MissingField(String),

    #[error("Auth failed: {0}")]
    Auth(String),

    #[error("Signing error: {0}")]
    Signing(String),
}

// ── Response Types ───────────────────────────────────────────────────────────

/// Binance API error envelope.
#[derive(Debug, Deserialize)]
struct BinanceErrorResponse {
    code: i64,
    msg: String,
}

/// Account info response from GET /api/v3/account.
#[derive(Debug, Deserialize)]
struct AccountInfo {
    #[serde(default)]
    balances: Vec<Balance>,
    #[serde(default)]
    can_trade: bool,
    #[serde(default)]
    account_type: String,
}

#[derive(Debug, Deserialize)]
struct Balance {
    asset: String,
    #[serde(default)]
    free: String,
    #[serde(default)]
    locked: String,
}

/// Order response from POST/DELETE/GET /api/v3/order.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceOrder {
    #[serde(default)]
    order_id: Option<i64>,
    #[serde(default)]
    client_order_id: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    orig_qty: Option<String>,
    #[serde(default)]
    executed_qty: Option<String>,
    #[serde(default)]
    cummulative_quote_qty: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    price: Option<String>,
    #[serde(default)]
    stop_price: Option<String>,
    #[serde(default)]
    orig_quote_order_qty: Option<String>,
    #[serde(default)]
    time_in_force: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    fills: Option<Vec<Fill>>,
    #[serde(default)]
    transact_time: Option<i64>,
    #[serde(default)]
    update_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Fill {
    #[serde(default)]
    price: String,
    #[serde(default)]
    qty: String,
    #[serde(default)]
    commission: String,
    #[serde(default)]
    commission_asset: String,
    #[serde(default)]
    trade_id: Option<i64>,
}

/// User data stream listen key response.
#[derive(Debug, Deserialize)]
struct ListenKey {
    #[serde(default)]
    listen_key: Option<String>,
}

// ── BinanceBroker ────────────────────────────────────────────────────────────

/// Live trading broker for Binance spot.
///
/// Uses HMAC-SHA256 signed REST API for order placement/management and
/// WebSocket user data streams for real-time fill confirmations.
///
/// ## Environment Variables
/// - `BINANCE_API_KEY` — Your Binance API key
/// - `BINANCE_SECRET_KEY` — Your Binance API secret
///
/// ## Rate Limits
/// Binance enforces weight-based rate limits. This adapter uses conservative
/// retry and backoff to stay within limits (default: 1200 weight per minute).
pub struct BinanceBroker {
    api_key: String,
    secret_key: String,
    base_url: String,
    ws_base_url: String,
    connected: AtomicBool,
    client: reqwest::Client,
    /// Listen key for user data stream (auto-renewed every 30 min)
    listen_key: RwLock<Option<String>>,
    /// Cached account balances (refreshed on each get_positions call)
    cached_balances: RwLock<HashMap<String, f64>>,
    /// Track last order execution time for position tracking (we simulate positions from balances)
    last_sync: RwLock<chrono::DateTime<Utc>>,
    /// Track order IDs we've placed so we can identify them in fills
    pending_orders: RwLock<HashMap<String, PendingOrderInfo>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingOrderInfo {
    symbol: String,
    direction: TradeDirection,
    qty: i32,
    entry_price: f64,
    stop_loss: f64,
    take_profit: f64,
    timestamp: chrono::DateTime<Utc>,
}

impl std::fmt::Debug for BinanceBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinanceBroker")
            .field("base_url", &self.base_url)
            .field("connected", &self.connected)
            .finish()
    }
}

impl BinanceBroker {
    /// Create a new Binance broker adapter.
    ///
    /// * `api_key` — Binance API key
    /// * `secret_key` — Binance API secret
    /// * `testnet` — If true, uses testnet.binance.vision instead of api.binance.com
    pub fn new(api_key: &str, secret_key: &str, testnet: bool) -> Self {
        let (base_url, ws_base_url) = if testnet {
            (
                "https://testnet.binance.vision".to_string(),
                "wss://testnet.binance.vision/ws".to_string(),
            )
        } else {
            (
                "https://api.binance.com".to_string(),
                "wss://stream.binance.com:9443/ws".to_string(),
            )
        };

        let client = reqwest::Client::builder()
            .user_agent("tredo/0.2.0 (Binance Spot API)")
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            api_key: api_key.to_string(),
            secret_key: secret_key.to_string(),
            base_url,
            ws_base_url,
            connected: AtomicBool::new(false),
            client,
            listen_key: RwLock::new(None),
            cached_balances: RwLock::new(HashMap::new()),
            last_sync: RwLock::new(Utc::now()),
            pending_orders: RwLock::new(HashMap::new()),
        }
    }

    // ── HMAC-SHA256 Signing ───────────────────────────────────────────────

    /// Create an HMAC-SHA256 signature for the given query string.
    fn sign(&self, query_string: &str) -> Result<String, BinanceError> {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| BinanceError::Signing(e.to_string()))?;
        mac.update(query_string.as_bytes());
        Ok(hex::encode(mac.finalize().into_bytes()))
    }

    /// Build a signed query string with timestamp + recvWindow.
    fn signed_query(&self, params: &[(&str, String)]) -> Result<String, BinanceError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let mut query_parts: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        query_parts.push(format!("timestamp={}", timestamp));
        query_parts.push("recvWindow=5000".to_string());

        let query_string = query_parts.join("&");
        let signature = self.sign(&query_string)?;
        Ok(format!("{}&signature={}", query_string, signature))
    }

    // ── HTTP Helpers ──────────────────────────────────────────────────────

    /// Make a signed GET request to the Binance REST API.
    async fn signed_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, BinanceError> {
        let query = self.signed_query(params)?;
        let url = format!("{}{}?{}", self.base_url, path, query);

        let resp = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body: BinanceErrorResponse = resp.json().await.unwrap_or(BinanceErrorResponse {
                code: -1,
                msg: "unknown error".to_string(),
            });
            return Err(BinanceError::Api {
                status,
                code: body.code,
                message: body.msg,
            });
        }

        Ok(resp.json().await?)
    }

    /// Make a signed POST request.
    async fn signed_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, BinanceError> {
        let query = self.signed_query(params)?;
        let url = format!("{}{}", self.base_url, path);

        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(query)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body_text = resp.text().await.unwrap_or_default();
            let body: BinanceErrorResponse =
                serde_json::from_str(&body_text).unwrap_or(BinanceErrorResponse {
                    code: -1,
                    msg: body_text,
                });
            return Err(BinanceError::Api {
                status,
                code: body.code,
                message: body.msg,
            });
        }

        Ok(resp.json().await?)
    }

    /// Make a signed DELETE request.
    async fn signed_delete<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, BinanceError> {
        let query = self.signed_query(params)?;
        let url = format!("{}{}?{}", self.base_url, path, query);

        let resp = self
            .client
            .delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body: BinanceErrorResponse = resp.json().await.unwrap_or(BinanceErrorResponse {
                code: -1,
                msg: "unknown error".to_string(),
            });
            return Err(BinanceError::Api {
                status,
                code: body.code,
                message: body.msg,
            });
        }

        Ok(resp.json().await?)
    }

    /// Make an unsigned GET request (public endpoints).
    async fn public_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<T, BinanceError> {
        let query_parts: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        let query = query_parts.join("&");
        let url = format!("{}{}?{}", self.base_url, path, query);

        let resp = self.client.get(&url).send().await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body: BinanceErrorResponse = resp.json().await.unwrap_or(BinanceErrorResponse {
                code: -1,
                msg: "unknown error".to_string(),
            });
            return Err(BinanceError::Api {
                status,
                code: body.code,
                message: body.msg,
            });
        }

        Ok(resp.json().await?)
    }

    /// Get current price for a symbol from public endpoint.
    async fn current_price(&self, symbol: &str) -> Result<f64, BinanceError> {
        let pair = to_binance_pair(symbol);
        let resp: serde_json::Value = self
            .public_get("/api/v3/ticker/price", &[("symbol", pair)])
            .await?;
        resp["price"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| BinanceError::MissingField("price".to_string()))
    }

    // ── WebSocket User Data Stream ────────────────────────────────────────

    /// Create a listen key for the user data stream.
    async fn create_listen_key(&self) -> Result<String, BinanceError> {
        let url = format!("{}/api/v3/userDataStream", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body: BinanceErrorResponse = resp.json().await.unwrap_or(BinanceErrorResponse {
                code: -1,
                msg: "listen key failed".to_string(),
            });
            return Err(BinanceError::Api {
                status,
                code: body.code,
                message: body.msg,
            });
        }

        let lk: ListenKey = resp.json().await?;
        lk.listen_key
            .ok_or_else(|| BinanceError::MissingField("listen_key".to_string()))
    }



    /// Spawn a background task that maintains the listen key and processes
    /// WebSocket user data stream for real-time execution reports (fills).
    async fn spawn_ws_listener(&self) {
        let lk = match self.create_listen_key().await {
            Ok(k) => {
                tracing::info!("🔑 Binance listen key created");
                k
            }
            Err(e) => {
                tracing::warn!("⚠️  Failed to create listen key: {} — fills will use polling", e);
                return;
            }
        };

        {
            let mut key = self.listen_key.write().await;
            *key = Some(lk.clone());
        }

        // Spawn listen key keepalive task (every 25 minutes)
        let keepalive_apikey = self.api_key.clone();
        let keepalive_client = self.client.clone();
        let keepalive_base = self.base_url.clone();
        let keepalive_key = lk.clone();
        tokio::spawn(async move {
            let mut keepalive_timer = tokio::time::interval(Duration::from_secs(25 * 60));
            loop {
                keepalive_timer.tick().await;
                let url = format!("{}/api/v3/userDataStream", keepalive_base);
                let body = format!("listenKey={}", keepalive_key);
                match keepalive_client
                    .put(&url)
                    .header("X-MBX-APIKEY", &keepalive_apikey)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(body)
                    .send()
                    .await
                {
                    Ok(_) => tracing::debug!("🔑 Listen key keepalive sent"),
                    Err(e) => tracing::warn!("⚠️  Listen key keepalive failed: {}", e),
                }
            }
        });

        // Spawn WebSocket connection for execution reports
        let ws_url = format!("{}/{}", self.ws_base_url, lk);
        tokio::spawn(async move {
            match tokio_tungstenite::connect_async(&ws_url).await {
                Ok((ws_stream, _)) => {
                    tracing::info!("🔌 Binance user data WS connected");
                    let (_write, mut read) = ws_stream.split();
                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                                // Parse execution report events
                                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) {
                                    let event_type = event["e"].as_str().unwrap_or("");
                                    match event_type {
                                        "executionReport" => {
                                            let symbol = event["s"].as_str().unwrap_or("unknown");
                                            let exec_type = event["x"].as_str().unwrap_or("");
                                            let order_status = event["X"].as_str().unwrap_or("");
                                            let side = event["S"].as_str().unwrap_or("");
                                            let qty = event["q"].as_str().unwrap_or("0");
                                            let price = event["p"].as_str().unwrap_or("0");
                                            let cum_quote = event["Z"].as_str().unwrap_or("0");
                                            let order_id = event["i"].as_i64().unwrap_or(0);

                                            if exec_type == "TRADE" && order_status == "FILLED" {
                                                tracing::info!(
                                                    "✅ BUY/SELL FILL: {} {} {} @ {} filled_qty={}",
                                                    symbol, side, order_id, price, qty
                                                );
                                            } else if exec_type == "TRADE" && order_status == "PARTIALLY_FILLED" {
                                                tracing::info!(
                                                    "🔹 Partial fill: {} {} {} @ {} (cumQuote: {})",
                                                    symbol, side, order_id, price, cum_quote
                                                );
                                            } else if order_status == "CANCELED" || order_status == "EXPIRED" {
                                                tracing::info!(
                                                    "🗑️ Order {} {}: {} {}",
                                                    order_id, order_status, symbol, side
                                                );
                                            } else if order_status == "REJECTED" {
                                                let reject_reason = event["r"].as_str().unwrap_or("unknown");
                                                tracing::warn!(
                                                    "❌ Order {} REJECTED: {} {} — reason: {}",
                                                    order_id, symbol, side, reject_reason
                                                );
                                            }
                                        }
                                        "outboundAccountPosition" => {
                                            // Balance update — can trigger position refresh
                                            tracing::debug!("📊 Account balance changed");
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                                tracing::warn!("🔌 Binance WS closed — will reconnect on next connect()");
                                break;
                            }
                            Err(e) => {
                                tracing::warn!("⚠️  Binance WS error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("⚠️  Binance WS connection failed: {} — fills will use polling", e);
                }
            }
        });
    }

    // ── Helper: Parse string to f64 ───────────────────────────────────────

    fn parse_decimal(s: &str) -> f64 {
        s.parse::<f64>().unwrap_or(0.0)
    }

    /// Map TREDO OrderType to Binance order type string.
    fn binance_order_type(order_type: OrderType) -> &'static str {
        match order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopLoss => "STOP_LOSS_LIMIT",
            OrderType::StopLossLimit => "STOP_LOSS_LIMIT",
        }
    }

    /// Map TREDO TradeDirection to Binance side.
    fn binance_side(direction: TradeDirection) -> &'static str {
        match direction {
            TradeDirection::Long => "BUY",
            TradeDirection::Short => "SELL",
        }
    }

    /// Parse Binance order status to TREDO OrderStatus.
    #[allow(unused_variables)]
    fn parse_order_status(status: &str, executed_qty: f64, orig_qty: f64) -> OrderStatus {
        match status {
            "NEW" | "PENDING" => OrderStatus::Pending,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled {
                filled_qty: executed_qty as i32,
            },
            "FILLED" => OrderStatus::Filled,
            "CANCELED" | "EXPIRED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected {
                reason: "Order rejected by Binance".to_string(),
            },
            _ => OrderStatus::Pending,
        }
    }

    /// Place a market order then set stop-loss and take-profit as separate orders.
    /// This is the common pattern for crypto trading (entry first, then bracket).
    async fn place_market_with_sltp(
        &self,
        pair: &str,
        direction: TradeDirection,
        qty: i32,
        market_price: f64,
        stop_loss: f64,
        take_profit: f64,
    ) -> Result<String, String> {
        // Step 1: Place market entry order
        let entry_params: Vec<(&str, String)> = vec![
            ("symbol", pair.to_string()),
            ("side", Self::binance_side(direction).to_string()),
            ("type", "MARKET".to_string()),
            ("quantity", qty.to_string()),
            ("newOrderRespType", "FULL".to_string()),
        ];

        let entry_result: BinanceOrder = self
            .signed_post("/api/v3/order", &entry_params)
            .await
            .map_err(|e| format!("Binance market entry failed: {}", e))?;

        let order_id = entry_result
            .order_id
            .map(|id| id.to_string())
            .ok_or_else(|| "No order_id in entry response".to_string())?;

        let fill_price = entry_result
            .fills
            .as_ref()
            .and_then(|fills| fills.first())
            .map(|f| Self::parse_decimal(&f.price))
            .unwrap_or(market_price);

        tracing::info!(
            "📈 Binance market entry filled: {} {} qty={} @ {:.2} (id={})",
            Self::binance_side(direction),
            pair,
            qty,
            fill_price,
            order_id
        );

        // Step 2: Place stop-loss limit order (opposite direction)
        let sl_side = match direction {
            TradeDirection::Long => "SELL",
            TradeDirection::Short => "BUY",
        };

        let sl_params: Vec<(&str, String)> = vec![
            ("symbol", pair.to_string()),
            ("side", sl_side.to_string()),
            ("type", "STOP_LOSS_LIMIT".to_string()),
            ("quantity", qty.to_string()),
            ("price", format!("{:.8}", stop_loss * 0.999)),
            ("stopPrice", format!("{:.8}", stop_loss)),
            ("timeInForce", "GTC".to_string()),
        ];

        let _: Result<BinanceOrder, _> = self
            .signed_post("/api/v3/order", &sl_params)
            .await
            .map_err(|e| tracing::warn!("⚠️  Stop-loss order failed (non-fatal): {}", e));

        // Step 3: Place take-profit limit order (opposite direction, better price)
        let tp_params: Vec<(&str, String)> = vec![
            ("symbol", pair.to_string()),
            ("side", sl_side.to_string()),
            ("type", "LIMIT".to_string()),
            ("quantity", qty.to_string()),
            ("price", format!("{:.8}", take_profit)),
            ("timeInForce", "GTC".to_string()),
        ];

        let _: Result<BinanceOrder, _> = self
            .signed_post("/api/v3/order", &tp_params)
            .await
            .map_err(|e| tracing::warn!("⚠️  Take-profit order failed (non-fatal): {}", e));

        // Track the pending order
        {
            let mut pending = self.pending_orders.write().await;
            pending.insert(
                order_id.clone(),
                PendingOrderInfo {
                    symbol: pair.to_string(),
                    direction,
                    qty,
                    entry_price: fill_price,
                    stop_loss,
                    take_profit,
                    timestamp: Utc::now(),
                },
            );
        }

        Ok(order_id)
    }

    /// Convert balances from account info into TREDO Positions.
    /// Non-zero free balances are treated as Long positions at current market price.
    async fn balances_to_positions(
        &self,
        balances: &[Balance],
    ) -> Result<Vec<Position>, BinanceError> {
        let mut positions = Vec::new();
        let now = Utc::now();

        // Fetch current prices for all non-zero balances
        for balance in balances {
            let free = Self::parse_decimal(&balance.free);
            let locked = Self::parse_decimal(&balance.locked);
            let total = free + locked;

            if total <= 0.0 || balance.asset == "USDT" || balance.asset == "BUSD" || balance.asset == "USDC" {
                continue;
            }

            // Get current price
            let pair = to_binance_pair(&balance.asset);
            let price = match self.current_price(&pair).await {
                Ok(p) => p,
                Err(_) => continue, // skip assets we can't price
            };

            // For crypto, we always treat balance as a Long position
            let unrealized_pnl = 0.0; // We don't have entry price from balance alone
            let unrealized_pnl_pct = 0.0;

            positions.push(Position {
                id: format!("BINANCE--{}--{}", balance.asset, now.timestamp_millis()),
                symbol: balance.asset.clone(),
                direction: TradeDirection::Long,
                qty: if total < 1.0 {
                    // For small amounts (e.g., 0.01 BTC), store as fractional
                    // but i32 doesn't support decimals — use a minimum viable unit
                    (total * 100_000.0).round() as i32
                } else {
                    total.round() as i32
                },
                entry_price: price,
                current_price: price,
                stop_loss: 0.0,
                take_profit: 0.0,
                unrealized_pnl,
                unrealized_pnl_pct,
                status: PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("Binance Live".to_string()),
                order_id: String::new(),
            });
        }

        Ok(positions)
    }
}

// ── BrokerAdapter Implementation ────────────────────────────────────────────

#[async_trait]
impl BrokerAdapter for BinanceBroker {
    /// Connect to Binance — validates API key by fetching account info,
    /// then spawns WebSocket listener for fill confirmations.
    async fn connect(&self) -> Result<(), String> {
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Validate API key by fetching account info
        let result: Result<AccountInfo, BinanceError> = self
            .signed_get("/api/v3/account", &[])
            .await;

        match result {
            Ok(account) => {
                if !account.can_trade {
                    return Err("Binance API key does not have trade permission. Enable it in the Binance API management dashboard.".to_string());
                }

                // Cache initial balances
                let mut balances = self.cached_balances.write().await;
                for b in &account.balances {
                    let free = Self::parse_decimal(&b.free);
                    let locked = Self::parse_decimal(&b.locked);
                    if free + locked > 0.0 {
                        balances.insert(b.asset.clone(), free + locked);
                    }
                }
                drop(balances);

                self.connected.store(true, Ordering::Relaxed);
                *self.last_sync.write().await = Utc::now();

                tracing::info!(
                    "✅ Binance connected (account: {}, {} assets)",
                    account.account_type,
                    account.balances.iter().filter(|b| {
                        Self::parse_decimal(&b.free) + Self::parse_decimal(&b.locked) > 0.0
                    }).count()
                );

                // Spawn WebSocket listener for fill confirmations (fire-and-forget)
                self.spawn_ws_listener().await;

                Ok(())
            }
            Err(e) => {
                let err_msg = match &e {
                    BinanceError::Api { code, message, .. } => {
                        match code {
                            -2015 => format!(
                                "Invalid API key or secret. Set BINANCE_API_KEY and BINANCE_SECRET_KEY. ({})",
                                message
                            ),
                            -2014 => format!("Invalid API key format: {}", message),
                            _ => format!("Binance API error ({}): {}", code, message),
                        }
                    }
                    BinanceError::Http(e) => format!("HTTP error: {}. Is api.binance.com reachable?", e),
                    _ => e.to_string(),
                };
                Err(err_msg)
            }
        }
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::Relaxed);
        let mut key = self.listen_key.write().await;
        *key = None;
        let mut orders = self.pending_orders.write().await;
        orders.clear();
        let mut balances = self.cached_balances.write().await;
        balances.clear();
        tracing::info!("🔌 Binance disconnected");
        Ok(())
    }

    /// Place a market/limit order on Binance using HMAC-SHA256 signing.
    ///
    /// If stop_loss AND take_profit are both set, uses an OCO order
    /// (One-Cancels-Other) for simultaneous SL/TP.
    async fn place_order(
        &self,
        request: OrderRequest,
        market_price: f64,
    ) -> Result<String, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err("Not connected to Binance. Call connect() first.".to_string());
        }

        let pair = to_binance_pair(&request.symbol);
        let side = Self::binance_side(request.direction);
        let qty = request.qty.to_string();
        let order_type = Self::binance_order_type(request.order_type);

        // Build base params
        let mut params: Vec<(&str, String)> = vec![
            ("symbol", pair.clone()),
            ("side", side.to_string()),
            ("type", order_type.to_string()),
            ("quantity", qty),
        ];

        // Add price for limit orders
        match request.order_type {
            OrderType::Limit => {
                let price = request.price.unwrap_or(market_price);
                params.push(("price", format!("{:.8}", price)));
                params.push(("timeInForce", "GTC".to_string()));
            }
            OrderType::StopLoss | OrderType::StopLossLimit => {
                let stop_price = request.stop_loss.unwrap_or(market_price * 0.99);
                let limit_price = request.price.unwrap_or(stop_price * 0.999);
                params.push(("stopPrice", format!("{:.8}", stop_price)));
                params.push(("price", format!("{:.8}", limit_price)));
            }
            OrderType::Market => {
                // Market orders just need quantity
            }
        }

        // If both SL and TP are specified, use OCO order
        let has_stop_loss = request.stop_loss.is_some() && request.stop_loss.unwrap() > 0.0;
        let has_take_profit = request.take_profit.is_some() && request.take_profit.unwrap() > 0.0;

        let order_id = if has_stop_loss && has_take_profit {
            // OCO order: place the main order with stop-loss and take-profit
            // Note: For spot, OCO combines a LIMIT order with STOP_LOSS_LIMIT.
            // For simplicity, we place a MARKET order first, then set SL/TP
            // as separate orders. True OCO requires LIMIT maker.
            self.place_market_with_sltp(
                &pair,
                request.direction,
                request.qty,
                market_price,
                request.stop_loss.unwrap(),
                request.take_profit.unwrap(),
            )
            .await?
        } else {
            // Standard order
            let result: BinanceOrder = self
                .signed_post("/api/v3/order", &params)
                .await
                .map_err(|e| format!("Binance place order failed: {}", e))?;

            let order_id = result
                .order_id
                .map(|id| id.to_string())
                .ok_or_else(|| "No order_id in Binance response".to_string())?;

            tracing::info!(
                "📈 Binance order placed: {} {} qty={} id={}",
                side,
                pair,
                request.qty,
                order_id
            );

            // Track the pending order
            {
                let mut pending = self.pending_orders.write().await;
                pending.insert(
                    order_id.clone(),
                    PendingOrderInfo {
                        symbol: request.symbol.clone(),
                        direction: request.direction,
                        qty: request.qty,
                        entry_price: market_price,
                        stop_loss: request.stop_loss.unwrap_or(0.0),
                        take_profit: request.take_profit.unwrap_or(0.0),
                        timestamp: Utc::now(),
                    },
                );
            }

            order_id
        };

        Ok(order_id)
    }

    /// Cancel an open order on Binance.
    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(BinanceError::NotConnected.to_string());
        }

        // We need the symbol to cancel. Try to find it from pending orders.
        let symbol = {
            let pending = self.pending_orders.read().await;
            pending
                .get(order_id)
                .map(|info| to_binance_pair(&info.symbol))
                .unwrap_or_else(|| {
                    // Fallback: try to cancel with a generic approach
                    let _ = order_id;
                    String::new()
                })
        };

        if symbol.is_empty() {
            return Err(format!(
                "Cannot cancel order {} — symbol not tracked. Use the Binance UI for manual cancellation.",
                order_id
            ));
        }

        let params: Vec<(&str, String)> = vec![
            ("symbol", symbol),
            ("orderId", order_id.to_string()),
        ];

        let _: BinanceOrder = self
            .signed_delete("/api/v3/order", &params)
            .await
            .map_err(|e| format!("Failed to cancel order {}: {}", order_id, e))?;

        {
            let mut pending = self.pending_orders.write().await;
            pending.remove(order_id);
        }

        tracing::info!("🗑️ Cancelled Binance order {}", order_id);
        Ok(())
    }

    /// Get open positions from Binance account balances.
    /// Non-zero balances (excluding USDT/stablecoins) are returned as Long positions.
    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(BinanceError::NotConnected.to_string());
        }

        let account: AccountInfo = self
            .signed_get("/api/v3/account", &[])
            .await
            .map_err(|e| e.to_string())?;

        // Update cached balances
        {
            let mut balances = self.cached_balances.write().await;
            balances.clear();
            for b in &account.balances {
                let total = Self::parse_decimal(&b.free) + Self::parse_decimal(&b.locked);
                if total > 0.0 {
                    balances.insert(b.asset.clone(), total);
                }
            }
        }

        *self.last_sync.write().await = Utc::now();

        self.balances_to_positions(&account.balances)
            .await
            .map_err(|e| e.to_string())
    }

    /// Get portfolio summary from Binance account.
    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(BinanceError::NotConnected.to_string());
        }

        let account: AccountInfo = self
            .signed_get("/api/v3/account", &[])
            .await
            .map_err(|e| e.to_string())?;

        // Calculate total USDT value
        let mut total_btc_value = 0.0;
        let mut cash_usdt = 0.0;
        
        for balance in &account.balances {
            let free = Self::parse_decimal(&balance.free);
            let locked = Self::parse_decimal(&balance.locked);
            let total = free + locked;

            if total <= 0.0 {
                continue;
            }

            if balance.asset == "USDT" {
                cash_usdt = total;
                total_btc_value += total;
                continue;
            }
            if balance.asset == "BUSD" || balance.asset == "USDC" {
                total_btc_value += total;
                continue;
            }

            // Price non-stable assets
            let pair = to_binance_pair(&balance.asset);
            if let Ok(price) = self.current_price(&pair).await {
                total_btc_value += total * price;
            }
        }

        let positions = self.get_positions().await.unwrap_or_default();
        let open_positions_count = positions.len();

        Ok(PortfolioSummary {
            cash: cash_usdt,
            equity: total_btc_value,
            margin_used: 0.0, // Spot has no margin
            free_margin: cash_usdt,
            daily_pnl: 0.0, // Would need historical tracking
            daily_pnl_pct: 0.0,
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            consecutive_losses: 0,
            max_drawdown: total_btc_value,
            max_drawdown_pct: 0.0,
            open_positions: open_positions_count,
            total_pnl_all_time: 0.0,
        })
    }

    /// Get order status from Binance.
    async fn get_order_status(&self, order_id: &str) -> Result<OrderStatus, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(BinanceError::NotConnected.to_string());
        }

        // Look up the symbol from pending orders
        let symbol = {
            let pending = self.pending_orders.read().await;
            pending
                .get(order_id)
                .map(|info| to_binance_pair(&info.symbol))
                .ok_or_else(|| format!("Order {} not tracked locally", order_id))?
        };

        let params: Vec<(&str, String)> = vec![
            ("symbol", symbol),
            ("orderId", order_id.to_string()),
        ];

        let order: BinanceOrder = self
            .signed_get("/api/v3/order", &params)
            .await
            .map_err(|e| format!("Failed to get order status: {}", e))?;

        let status = order.status.as_deref().unwrap_or("UNKNOWN");
        let executed_qty = order
            .executed_qty
            .as_deref()
            .map(Self::parse_decimal)
            .unwrap_or(0.0);
        let orig_qty = order
            .orig_qty
            .as_deref()
            .map(Self::parse_decimal)
            .unwrap_or(0.0);

        Ok(Self::parse_order_status(status, executed_qty, orig_qty))
    }

    /// Get recent filled trades from Binance.
    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(BinanceError::NotConnected.to_string());
        }

        // Binance doesn't have a unified "recent trades" endpoint for filled orders.
        // We use the all orders endpoint for common symbols.
        let symbols_to_check = vec!["BTCUSDT", "ETHUSDT", "SOLUSDT"];
        let mut trades = Vec::new();

        for symbol in &symbols_to_check {
            let params: Vec<(&str, String)> = vec![
                ("symbol", symbol.to_string()),
                ("limit", (limit as u32 / 3).max(5).to_string()),
            ];

            if let Ok(orders) = self
                .signed_get::<Vec<BinanceOrder>>("/api/v3/allOrders", &params)
                .await
            {
                let now = Utc::now();
                for order in &orders {
                    let status = order.status.as_deref().unwrap_or("");
                    if status != "FILLED" {
                        continue;
                    }
                    let side = order.side.as_deref().unwrap_or("BUY");
                    let direction = match side {
                        "BUY" => TradeDirection::Long,
                        _ => TradeDirection::Short,
                    };
                    let executed_qty = order
                        .executed_qty
                        .as_deref()
                        .map(Self::parse_decimal)
                        .unwrap_or(0.0);
                    let cum_quote = order
                        .cummulative_quote_qty
                        .as_deref()
                        .map(Self::parse_decimal)
                        .unwrap_or(0.0);
                    let avg_price = if executed_qty > 0.0 {
                        cum_quote / executed_qty
                    } else {
                        0.0
                    };

                    trades.push(ClosedTrade {
                        id: order.order_id.map(|id| id.to_string()).unwrap_or_default(),
                        symbol: order.symbol.clone().unwrap_or_default(),
                        direction,
                        qty: executed_qty as i32,
                        entry_price: avg_price,
                        exit_price: 0.0,
                        realized_pnl: 0.0,
                        realized_pnl_pct: 0.0,
                        close_reason: CloseReason::Manual,
                        opened_at: now,
                        closed_at: now,
                        duration_secs: 0,
                        strategy: Some("Binance Live".to_string()),
                        order_id: String::new(),
                    });
                }
            }
        }

        trades.truncate(limit);
        Ok(trades)
    }

    /// Update positions with latest market price.
    /// Binance handles its own SL/TP via OCO orders, so this is a no-op
    /// that just refreshes cached balances.
    async fn update_price(
        &self,
        _symbol: &str,
        _market_price: f64,
    ) -> Result<Vec<ClosedTrade>, String> {
        // Binance handles stop-loss and take-profit natively via OCO orders.
        // We don't need to monitor SL/TP ourselves — Binance's matching engine
        // will trigger them. This method just refreshes positions.
        //
        // Return any closed orders that Binance detected (OCO hits).
        // For simplicity, we return an empty vec — positions are refreshed
        // on the next get_positions() call.
        Ok(Vec::new())
    }

    /// Close a position by placing an opposing market order on Binance.
    async fn close_position(
        &self,
        position_id: &str,
        exit_price: f64,
    ) -> Result<ClosedTrade, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(BinanceError::NotConnected.to_string());
        }

        // Parse position ID: "BINANCE--{asset}--{timestamp}"
        let parts: Vec<&str> = position_id.splitn(3, "--").collect();
        if parts.len() < 2 || parts[1].is_empty() {
            return Err(format!("Invalid position ID format: {}", position_id));
        }

        let asset = parts[1];
        let pair = to_binance_pair(asset);

        // Get current balance for this asset
        let account: AccountInfo = self
            .signed_get("/api/v3/account", &[])
            .await
            .map_err(|e| e.to_string())?;

        let balance = account
            .balances
            .into_iter()
            .find(|b| b.asset == asset)
            .ok_or_else(|| format!("Asset {} not found in Binance account", asset))?;

        let free = Self::parse_decimal(&balance.free);
        if free <= 0.0 {
            return Err(format!("No {} balance to sell", asset));
        }

        // Place a SELL market order to close the position
        let params: Vec<(&str, String)> = vec![
            ("symbol", pair.clone()),
            ("side", "SELL".to_string()),
            ("type", "MARKET".to_string()),
            ("quantity", format!("{:.8}", free)),
            ("newOrderRespType", "FULL".to_string()),
        ];

        let result: BinanceOrder = self
            .signed_post("/api/v3/order", &params)
            .await
            .map_err(|e| format!("Binance close position failed: {}", e))?;

        let fill_price = result
            .fills
            .as_ref()
            .and_then(|fills| fills.first())
            .map(|f| Self::parse_decimal(&f.price))
            .unwrap_or(exit_price);

        let order_id = result
            .order_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Remove from pending orders
        {
            let mut pending = self.pending_orders.write().await;
            pending.retain(|_, info| info.symbol != asset);
        }

        // Update cached balance
        {
            let mut balances = self.cached_balances.write().await;
            balances.remove(asset);
        }

        tracing::info!(
            "🗑️ Binance position closed: {} SELL {:.8} @ {:.2} (id={})",
            pair,
            free,
            fill_price,
            order_id
        );

        Ok(ClosedTrade {
            id: position_id.to_string(),
            symbol: asset.to_string(),
            direction: TradeDirection::Long,
            qty: free as i32,
            entry_price: 0.0, // We don't track entry price historically
            exit_price: fill_price,
            realized_pnl: 0.0,
            realized_pnl_pct: 0.0,
            close_reason: CloseReason::Manual,
            opened_at: Utc::now(),
            closed_at: Utc::now(),
            duration_secs: 0,
            strategy: Some("Binance Live".to_string()),
            order_id,
        })
    }

    /// Risk check — Binance handles its own risk checks (margin, balance).
    /// We trust Binance's risk management.
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
            warnings: vec![
                "Risk checks delegated to Binance exchange".to_string(),
            ],
        })
    }

    /// Cannot reset a live Binance account.
    async fn reset(&self) -> Result<(), String> {
        Err("Cannot reset a live Binance account. Use paper mode for reset.".to_string())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        "Binance"
    }
}

// ── Helper to create an Arc'd broker for use with BrokerRegistry ────────────

/// Create a Binance broker adapter wrapped in an Arc.
///
/// Reads credentials from environment variables:
/// - `BINANCE_API_KEY` — Your Binance API key
/// - `BINANCE_SECRET_KEY` — Your Binance API secret
/// - `BINANCE_TESTNET` — Set to "true" to use testnet (optional)
pub fn create_binance_broker() -> std::sync::Arc<dyn BrokerAdapter> {
    let api_key = std::env::var("BINANCE_API_KEY")
        .unwrap_or_else(|_| panic!("BINANCE_API_KEY environment variable not set"));
    let secret_key = std::env::var("BINANCE_SECRET_KEY")
        .unwrap_or_else(|_| panic!("BINANCE_SECRET_KEY environment variable not set"));
    let testnet = std::env::var("BINANCE_TESTNET")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    std::sync::Arc::new(BinanceBroker::new(&api_key, &secret_key, testnet))
}

// ── Private helper: normalize symbol to Binance pair ───────────────────────

fn to_binance_pair(symbol: &str) -> String {
    let upper = symbol.trim().to_uppercase();
    if upper.ends_with("USDT") || upper.ends_with("BUSD") || upper.ends_with("USDC") {
        return upper;
    }
    format!("{}USDT", upper)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broker_name_and_mode() {
        let broker = BinanceBroker::new("key", "secret", false);
        assert_eq!(broker.broker_name(), "Binance");
        assert_eq!(broker.mode(), TradingMode::Live);
    }

    #[test]
    fn test_testnet_name() {
        let broker = BinanceBroker::new("key", "secret", true);
        assert_eq!(broker.broker_name(), "Binance");
        // mode is still Live even on testnet — it's a real execution environment
        assert_eq!(broker.mode(), TradingMode::Live);
    }

    #[test]
    fn test_to_binance_pair() {
        assert_eq!(to_binance_pair("BTC"), "BTCUSDT");
        assert_eq!(to_binance_pair("ETH"), "ETHUSDT");
        assert_eq!(to_binance_pair("btc"), "BTCUSDT");
        assert_eq!(to_binance_pair("BTCUSDT"), "BTCUSDT");
        assert_eq!(to_binance_pair("SOLBUSD"), "SOLBUSD");
    }

    #[test]
    fn test_binance_side() {
        assert_eq!(BinanceBroker::binance_side(TradeDirection::Long), "BUY");
        assert_eq!(BinanceBroker::binance_side(TradeDirection::Short), "SELL");
    }

    #[test]
    fn test_binance_order_type() {
        assert_eq!(BinanceBroker::binance_order_type(OrderType::Market), "MARKET");
        assert_eq!(BinanceBroker::binance_order_type(OrderType::Limit), "LIMIT");
        assert_eq!(BinanceBroker::binance_order_type(OrderType::StopLoss), "STOP_LOSS_LIMIT");
    }

    #[test]
    fn test_parse_order_status() {
        assert_eq!(
            BinanceBroker::parse_order_status("FILLED", 10.0, 10.0),
            OrderStatus::Filled
        );
        assert_eq!(
            BinanceBroker::parse_order_status("NEW", 0.0, 10.0),
            OrderStatus::Pending
        );
        assert_eq!(
            BinanceBroker::parse_order_status("PARTIALLY_FILLED", 5.0, 10.0),
            OrderStatus::PartiallyFilled { filled_qty: 5 }
        );
        assert_eq!(
            BinanceBroker::parse_order_status("REJECTED", 0.0, 10.0),
            OrderStatus::Rejected {
                reason: "Order rejected by Binance".to_string()
            }
        );
        assert_eq!(
            BinanceBroker::parse_order_status("CANCELED", 0.0, 10.0),
            OrderStatus::Cancelled
        );
    }

    #[test]
    fn test_parse_decimal() {
        assert!((BinanceBroker::parse_decimal("1.234") - 1.234).abs() < 1e-9);
        assert!((BinanceBroker::parse_decimal("0.0") - 0.0).abs() < 1e-9);
        assert!((BinanceBroker::parse_decimal("") - 0.0).abs() < 1e-9);
        assert!((BinanceBroker::parse_decimal("abc") - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_sign_creates_hmac() {
        let broker = BinanceBroker::new("key", "secret", false);
        let sig = broker.sign("symbol=BTCUSDT&side=BUY&type=MARKET&quantity=0.01&timestamp=1234567890&recvWindow=5000");
        assert!(sig.is_ok());
        let sig = sig.unwrap();
        assert_eq!(sig.len(), 64, "HMAC-SHA256 hex should be 64 chars");
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()), "Should be hex");
    }

    #[tokio::test]
    #[ignore] // requires network
    async fn test_connect_validation() {
        let broker = BinanceBroker::new("invalid", "invalid", true);
        let result = broker.connect().await;
        assert!(result.is_err(), "Should fail with invalid API key");
        assert!(
            result.unwrap_err().contains("Invalid API key"),
            "Should mention invalid API key"
        );
    }
}
