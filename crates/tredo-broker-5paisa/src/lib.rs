//! # 5Paisa Xstream API — Free Indian Discount Broker Adapter
// Field names match the 5Paisa API's PascalCase JSON — suppress style warnings
#![allow(non_snake_case, dead_code)]

//!
//! Implements [`BrokerAdapter`] for trading via the 5Paisa Xstream REST API.
//! 5Paisa (formerly India Infoline / IIFL) is a popular free Indian discount broker.
//!
//! ## Authentication Flow
//! 1. Register app at https://xstream.5paisa.com/dev-docs/ to get `AppKey`, `EncryKey`, `UserId`
//! 2. User logs in via: `https://dev-openapi.5paisa.com/WebVendorLogin/VLogin/Index`
//!    with `VendorKey`, `ResponseURL`, `State` parameters
//! 3. Redirected to callback URL with a `RequestToken`
//! 4. Exchange `RequestToken` for an `AccessToken` (JWT, valid until 11:59 PM)
//! 5. All subsequent requests use `Authorization: bearer {access_token}`
//!
//! ## Environment Variables
//! - `FIVEPAISA_APP_KEY` — Your 5Paisa AppKey
//! - `FIVEPAISA_ENCRY_KEY` — Your encryption key
//! - `FIVEPAISA_USER_ID` — Your user ID
//! - `FIVEPAISA_CLIENT_CODE` — Your client code
//! - `FIVEPAISA_ACCESS_TOKEN` — Pre-obtained access token (skip login flow)

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
pub enum FivePaisaError {
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

// ── Response/Request Types ───────────────────────────────────────────────────

/// 5Paisa API header envelope (used in requests and responses).
#[derive(Debug, Serialize, Deserialize)]
struct FivePaisaHead {
    #[serde(default)]
    key: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    statusDescription: Option<String>,
}

/// Generic 5Paisa API response envelope.
#[derive(Debug, Deserialize)]
struct FivePaisaResponse<T> {
    head: Option<FivePaisaHead>,
    body: Option<T>,
}

/// Token exchange request
#[derive(Debug, Serialize)]
struct AccessTokenRequest {
    RequestToken: String,
    EncryKey: String,
    UserID: String,
}

/// Token exchange response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AccessTokenResponse {
    #[serde(default)]
    AccessToken: Option<String>,
    #[serde(default)]
    TokenType: Option<String>,
}

/// Place order request body
#[derive(Debug, Serialize)]
struct PlaceOrderBody {
    Exchange: String,
    ExchangeType: String,
    ScripCode: String,
    Price: String,
    StopLossPrice: String,
    OrderType: String,
    Qty: i32,
    DisQty: String,
    IsIntraday: bool,
    iOrderValidity: String,
    AHPlaced: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    RemoteOrderID: Option<String>,
}

/// Place order request wrapper
#[derive(Debug, Serialize)]
struct PlaceOrderRequest {
    head: FivePaisaHead,
    body: PlaceOrderBody,
}

/// Place order response body
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PlaceOrderResponseBody {
    #[serde(default)]
    BrokerOrderID: Option<String>,
    #[serde(default)]
    ClientCode: Option<String>,
    #[serde(default)]
    Exch: Option<String>,
    #[serde(default)]
    ExchOrderID: Option<String>,
    #[serde(default)]
    ExchType: Option<String>,
    #[serde(default)]
    Message: Option<String>,
    #[serde(default)]
    Status: Option<i32>,
    #[serde(default)]
    OrderRequesterID: Option<String>,
    #[serde(default)]
    RemoteOrderID: Option<String>,
}

/// Cancel order request body
#[derive(Debug, Serialize)]
struct CancelOrderBody {
    Exchange: String,
    ExchangeType: String,
    OrderID: String,
}

/// Cancel order request wrapper
#[derive(Debug, Serialize)]
struct CancelOrderRequest {
    head: FivePaisaHead,
    body: CancelOrderBody,
}

/// Position response data
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FivePaisaPosition {
    #[serde(default)]
    Exch: Option<String>,
    #[serde(default)]
    ExchType: Option<String>,
    #[serde(default)]
    ScripCode: Option<i32>,
    #[serde(default)]
    ScripName: Option<String>,
    #[serde(default)]
    NetQuantity: Option<i32>,
    #[serde(default)]
    BuyQuantity: Option<i32>,
    #[serde(default)]
    SellQuantity: Option<i32>,
    #[serde(default)]
    BEP: Option<f64>,
    #[serde(default)]
    LTP: Option<f64>,
    #[serde(default)]
    PnL: Option<f64>,
    #[serde(default)]
    MToM: Option<f64>,
    #[serde(default)]
    BuyAvgRate: Option<f64>,
    #[serde(default)]
    SellAvgRate: Option<f64>,
    #[serde(default)]
    Multiplier: Option<f64>,
}

/// Holding response data
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FivePaisaHolding {
    #[serde(default)]
    ScripCode: Option<i32>,
    #[serde(default)]
    ScripName: Option<String>,
    #[serde(default)]
    Exch: Option<String>,
    #[serde(default)]
    ExchType: Option<String>,
    #[serde(default)]
    Quantity: Option<i32>,
    #[serde(default)]
    AveragePrice: Option<f64>,
    #[serde(default)]
    LTP: Option<f64>,
    #[serde(default)]
    PnL: Option<f64>,
    #[serde(default)]
    Haircut: Option<f64>,
}

/// Order / trade book response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FivePaisaOrder {
    #[serde(default)]
    OrderID: Option<String>,
    #[serde(default)]
    ExchOrderID: Option<String>,
    #[serde(default)]
    ScripName: Option<String>,
    #[serde(default)]
    ScripCode: Option<i32>,
    #[serde(default)]
    Exch: Option<String>,
    #[serde(default)]
    ExchType: Option<String>,
    #[serde(default)]
    BuySell: Option<String>,
    #[serde(default)]
    Qty: Option<i32>,
    #[serde(default)]
    PendingQty: Option<i32>,
    #[serde(default)]
    FillQty: Option<i32>,
    #[serde(default)]
    AvgRate: Option<f64>,
    #[serde(default)]
    OrderStatus: Option<String>,
    #[serde(default)]
    OrderTime: Option<String>,
    #[serde(default)]
    RejectionReason: Option<String>,
}

/// Margin/balance response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FivePaisaMargin {
    #[serde(default)]
    TotalMargin: Option<f64>,
    #[serde(default)]
    UsedMargin: Option<f64>,
    #[serde(default)]
    AvailableMargin: Option<f64>,
    #[serde(default)]
    AvailableCash: Option<f64>,
    #[serde(default)]
    Collateral: Option<f64>,
    #[serde(default)]
    PayInAmount: Option<f64>,
}

/// Order status request body
#[derive(Debug, Serialize)]
struct OrderStatusBody {
    OrderID: String,
}

/// Order status request wrapper
#[derive(Debug, Serialize)]
struct OrderStatusRequest {
    head: FivePaisaHead,
    body: OrderStatusBody,
}

// ── FivePaisaBroker ──────────────────────────────────────────────────────────

/// Live trading broker for 5Paisa Xstream API.
pub struct FivePaisaBroker {
    app_key: String,
    encry_key: String,
    user_id: String,
    client_code: String,
    base_url: String,
    access_token: RwLock<Option<String>>,
    connected: AtomicBool,
    client: reqwest::Client,
}

impl std::fmt::Debug for FivePaisaBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FivePaisaBroker")
            .field("base_url", &self.base_url)
            .field("connected", &self.connected)
            .finish()
    }
}

impl FivePaisaBroker {
    /// Create a new 5Paisa broker.
    ///
    /// * `app_key` — Your 5Paisa AppKey (from developer portal)
    /// * `encry_key` — Your encryption key
    /// * `user_id` — Your user ID
    /// * `client_code` — Your client code
    /// * `access_token` — Pre-obtained access token (or empty)
    pub fn new(
        app_key: &str,
        encry_key: &str,
        user_id: &str,
        client_code: &str,
        access_token: &str,
    ) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("tredo/0.2.0 (5Paisa Xstream API)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            app_key: app_key.to_string(),
            encry_key: encry_key.to_string(),
            user_id: user_id.to_string(),
            client_code: client_code.to_string(),
            base_url: "https://Openapi.5paisa.com".to_string(),
            access_token: RwLock::new(if access_token.is_empty() {
                None
            } else {
                Some(access_token.to_string())
            }),
            connected: AtomicBool::new(false),
            client,
        }
    }

    /// Build the common headers with Authorization.
    fn headers(&self, token: Option<&str>) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        if let Some(token) = token {
            if let Ok(v) = HeaderValue::from_str(&format!("bearer {}", token)) {
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
    ) -> Result<T, FivePaisaError> {
        let url = format!("{}{}", self.base_url, path);
        let token = if requires_auth {
            Some(
                self.access_token
                    .read()
                    .await
                    .clone()
                    .ok_or(FivePaisaError::Auth("Not authenticated".into()))?,
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
            return Err(FivePaisaError::TokenExpired);
        }
        if !(200..300).contains(&status) {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(FivePaisaError::Api {
                status,
                message: body_text,
            });
        }

        // 5Paisa APIs return data either as a wrapped response or directly as body
        Ok(resp.json().await?)
    }

    /// Make an authenticated GET request.
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, FivePaisaError> {
        let url = format!("{}{}", self.base_url, path);
        let token = self
            .access_token
            .read()
            .await
            .clone()
            .ok_or(FivePaisaError::Auth("Not authenticated".into()))?;
        let headers = self.headers(Some(&token));

        let resp = self.client.get(&url).headers(headers).send().await?;

        let status = resp.status().as_u16();
        if status == 401 {
            return Err(FivePaisaError::TokenExpired);
        }
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(FivePaisaError::Api {
                status,
                message: body,
            });
        }

        Ok(resp.json().await?)
    }

    /// Map TREDO OrderType to 5Paisa order type string.
    fn tp_order_type(order_type: OrderType) -> &'static str {
        match order_type {
            OrderType::Market => "Market",
            OrderType::Limit => "Limit",
            OrderType::StopLoss | OrderType::StopLossLimit => "StopLoss",
        }
    }

    /// Map TREDO direction to 5Paisa Buy/Sell.
    fn tp_transaction_type(direction: TradeDirection) -> &'static str {
        match direction {
            TradeDirection::Long => "Buy",
            TradeDirection::Short => "Sell",
        }
    }

    /// Parse 5Paisa order status string.
    fn parse_order_status(status: &str) -> OrderStatus {
        match status.to_uppercase().as_str() {
            "PENDING" | "OPEN" | "TRIGGER_PENDING" => OrderStatus::Pending,
            "COMPLETE" | "FILLED" => OrderStatus::Filled,
            "CANCELLED" | "CANCELED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected {
                reason: "Order rejected by 5Paisa".into(),
            },
            _ => OrderStatus::Pending,
        }
    }

    /// Map Exchange string for 5Paisa.
    /// NOTE: branches are intentionally identical for now — both NSE and the
    /// default resolve to "N" until the master contract list is wired in.
    #[allow(clippy::if_same_then_else)]
    fn exchange_code(symbol: &str) -> &str {
        let sym = symbol.to_uppercase();
        if sym.ends_with(".NS") || sym == "NIFTY" || sym == "BANKNIFTY" || sym == "FINNIFTY" {
            "N"
        } else {
            "N" // Default NSE
        }
    }

    /// NOTE: branches intentionally identical until exchange-type resolution
    /// (Cash vs F&O) is implemented from the master contract list.
    #[allow(clippy::if_same_then_else)]
    fn exchange_type(symbol: &str) -> &str {
        let sym = symbol.to_uppercase();
        if sym.ends_with(".NS") || sym == "NIFTY" || sym == "BANKNIFTY" || sym == "FINNIFTY" {
            "C" // Cash / Equity
        } else {
            "C"
        }
    }

    /// ScripCode — in production, resolve from master contract list.
    /// Using 0 as sentinel: the API will reject it with an error message containing the correct code.
    fn resolve_scrip_code(_symbol: &str) -> String {
        "0".to_string()
    }
}

#[async_trait]
impl BrokerAdapter for FivePaisaBroker {
    async fn connect(&self) -> Result<(), String> {
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        // If we have an existing token, validate it
        {
            let token = self.access_token.read().await;
            if token.is_some() {
                // Validate by fetching available margin
                let url = format!("{}/VendorsAPI/Service1.svc/Margin", self.base_url);
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
                        tracing::info!("✅ 5Paisa connected (existing token)");
                        return Ok(());
                    }
                }
            }
        }

        // No valid token — provide instructions
        Err(format!(
            "5Paisa: No valid access token. Set FIVEPAISA_ACCESS_TOKEN or visit:\n  \
             https://dev-openapi.5paisa.com/WebVendorLogin/VLogin/Index?VendorKey={}&ResponseURL=YOUR_CALLBACK",
            self.app_key
        ))
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.connected.store(false, Ordering::Relaxed);
        let mut token = self.access_token.write().await;
        *token = None;
        tracing::info!("🔌 5Paisa disconnected");
        Ok(())
    }

    async fn place_order(
        &self,
        request: OrderRequest,
        _market_price: f64,
    ) -> Result<String, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(FivePaisaError::NotConnected.to_string());
        }

        let scrip_code = Self::resolve_scrip_code(&request.symbol);
        let remote_order_id = request
            .client_order_id
            .clone()
            .or_else(|| Some(format!("tredo_{}", chrono::Utc::now().timestamp_millis())));

        let req = PlaceOrderRequest {
            head: FivePaisaHead {
                key: self.app_key.clone(),
                status: None,
                statusDescription: None,
            },
            body: PlaceOrderBody {
                Exchange: Self::exchange_code(&request.symbol).to_string(),
                ExchangeType: Self::exchange_type(&request.symbol).to_string(),
                ScripCode: scrip_code,
                Price: match request.order_type {
                    OrderType::Market => "0".to_string(),
                    _ => request
                        .price
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "0".to_string()),
                },
                StopLossPrice: request
                    .stop_loss
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "0".to_string()),
                OrderType: Self::tp_transaction_type(request.direction).to_string(),
                Qty: request.qty,
                DisQty: "0".to_string(),
                IsIntraday: true,
                iOrderValidity: "0".to_string(), // DAY
                AHPlaced: "N".to_string(),
                RemoteOrderID: remote_order_id,
            },
        };

        let body = serde_json::to_value(&req).map_err(|e| format!("Serialization error: {}", e))?;

        let resp: FivePaisaResponse<PlaceOrderResponseBody> = self
            .post("/VendorsAPI/Service1.svc/V1/PlaceOrderRequest", &body, true)
            .await
            .map_err(|e| format!("5Paisa place order failed: {}", e))?;

        let order_body = resp
            .body
            .ok_or_else(|| "No body in place order response".to_string())?;

        let status_code = order_body.Status.unwrap_or(-1);
        if status_code != 0 {
            return Err(format!(
                "5Paisa order rejected: {}",
                order_body.Message.as_deref().unwrap_or("Unknown error")
            ));
        }

        let order_id = order_body
            .BrokerOrderID
            .or(order_body.ExchOrderID)
            .ok_or_else(|| "No order ID in response".to_string())?
            .to_string();

        tracing::info!(
            "📈 5Paisa order placed: {} {} qty={} id={}",
            Self::tp_transaction_type(request.direction),
            request.symbol,
            request.qty,
            order_id
        );

        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: &str) -> Result<(), String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(FivePaisaError::NotConnected.to_string());
        }

        let req = CancelOrderRequest {
            head: FivePaisaHead {
                key: self.app_key.clone(),
                status: None,
                statusDescription: None,
            },
            body: CancelOrderBody {
                Exchange: "N".to_string(),
                ExchangeType: "C".to_string(),
                OrderID: order_id.to_string(),
            },
        };

        let body = serde_json::to_value(&req).map_err(|e| format!("Serialization error: {}", e))?;

        let _: serde_json::Value = self
            .post("/VendorsAPI/Service1.svc/CancelOrderRequest", &body, true)
            .await
            .map_err(|e| format!("Failed to cancel order {}: {}", order_id, e))?;

        tracing::info!("🗑️ Cancelled 5Paisa order {}", order_id);
        Ok(())
    }

    async fn get_positions(&self) -> Result<Vec<Position>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(FivePaisaError::NotConnected.to_string());
        }

        let positions: FivePaisaResponse<Vec<FivePaisaPosition>> = self
            .get("/VendorsAPI/Service1.svc/V2/NetPositionNetWise")
            .await
            .map_err(|e| e.to_string())?;

        let holdings: FivePaisaResponse<Vec<FivePaisaHolding>> = self
            .get("/VendorsAPI/Service1.svc/V3/Holding")
            .await
            .map_err(|e| e.to_string())?;

        let now = Utc::now();
        let mut result = Vec::new();

        let pos_list = positions.body.unwrap_or_default();
        for pos in &pos_list {
            let net_qty = pos.NetQuantity.unwrap_or(0);
            if net_qty == 0 {
                continue;
            }
            let direction = if net_qty > 0 {
                TradeDirection::Long
            } else {
                TradeDirection::Short
            };
            let entry_price = pos.BEP.or(pos.BuyAvgRate).unwrap_or(0.0);
            let current_price = pos.LTP.unwrap_or(entry_price);
            let pnl = pos.PnL.or(pos.MToM).unwrap_or(0.0);
            let symbol = pos
                .ScripName
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            result.push(Position {
                id: format!("5PAISA--{}--{}", symbol, now.timestamp_millis()),
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
                strategy: Some("5Paisa Live".to_string()),
                order_id: String::new(),
            });
        }

        let hld_list = holdings.body.unwrap_or_default();
        for h in &hld_list {
            let qty = h.Quantity.unwrap_or(0);
            if qty == 0
                || result
                    .iter()
                    .any(|p| p.symbol == h.ScripName.as_deref().unwrap_or(""))
            {
                continue;
            }
            let symbol = h.ScripName.clone().unwrap_or_else(|| "UNKNOWN".to_string());
            result.push(Position {
                id: format!("5PAISA-HLDG--{}--{}", symbol, now.timestamp_millis()),
                symbol,
                direction: TradeDirection::Long,
                qty,
                entry_price: h.AveragePrice.unwrap_or(0.0),
                current_price: h.LTP.unwrap_or(0.0),
                stop_loss: 0.0,
                take_profit: 0.0,
                unrealized_pnl: h.PnL.unwrap_or(0.0),
                unrealized_pnl_pct: 0.0,
                status: PositionStatus::Open,
                opened_at: now,
                closed_at: None,
                strategy: Some("5Paisa Holdings".to_string()),
                order_id: String::new(),
            });
        }

        Ok(result)
    }

    async fn get_summary(&self) -> Result<PortfolioSummary, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(FivePaisaError::NotConnected.to_string());
        }

        let margin: FivePaisaResponse<FivePaisaMargin> = self
            .get("/VendorsAPI/Service1.svc/Margin")
            .await
            .map_err(|e| e.to_string())?;

        let margin_data = margin.body.unwrap_or(FivePaisaMargin {
            TotalMargin: None,
            UsedMargin: None,
            AvailableMargin: None,
            AvailableCash: None,
            Collateral: None,
            PayInAmount: None,
        });

        let cash = margin_data.AvailableCash.unwrap_or(0.0)
            + margin_data.Collateral.unwrap_or(0.0)
            + margin_data.PayInAmount.unwrap_or(0.0);
        let used = margin_data.UsedMargin.unwrap_or(0.0);
        let total_equity = margin_data.TotalMargin.unwrap_or(cash + used);

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
            return Err(FivePaisaError::NotConnected.to_string());
        }

        let req = OrderStatusRequest {
            head: FivePaisaHead {
                key: self.app_key.clone(),
                status: None,
                statusDescription: None,
            },
            body: OrderStatusBody {
                OrderID: order_id.to_string(),
            },
        };

        let body = serde_json::to_value(&req).map_err(|e| format!("Serialization error: {}", e))?;

        let order_resp: FivePaisaResponse<FivePaisaOrder> = self
            .post("/VendorsAPI/Service1.svc/OrderStatus", &body, true)
            .await
            .map_err(|e| e.to_string())?;

        let order = order_resp
            .body
            .ok_or_else(|| "No order data in response".to_string())?;

        let status = order.OrderStatus.unwrap_or_else(|| "PENDING".to_string());
        Ok(Self::parse_order_status(&status))
    }

    async fn get_recent_trades(&self, limit: usize) -> Result<Vec<ClosedTrade>, String> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(FivePaisaError::NotConnected.to_string());
        }

        let trades_resp: FivePaisaResponse<Vec<FivePaisaOrder>> = self
            .get("/VendorsAPI/Service1.svc/TradeBook")
            .await
            .map_err(|e| e.to_string())?;

        let now = Utc::now();
        let mut trades: Vec<ClosedTrade> = trades_resp
            .body
            .unwrap_or_default()
            .into_iter()
            .filter(|o| {
                o.OrderStatus.as_deref() == Some("COMPLETE")
                    || o.OrderStatus.as_deref() == Some("FILLED")
            })
            .take(limit)
            .map(|o| {
                let direction = match o.BuySell.as_deref() {
                    Some("Buy") => TradeDirection::Long,
                    _ => TradeDirection::Short,
                };
                let qty = o.FillQty.unwrap_or(0).abs();
                let price = o.AvgRate.unwrap_or(0.0);

                ClosedTrade {
                    id: o.OrderID.unwrap_or_default(),
                    symbol: o.ScripName.unwrap_or_default(),
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
                    strategy: Some("5Paisa Live".to_string()),
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
            return Err(FivePaisaError::NotConnected.to_string());
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
            strategy: Some("5Paisa Live".to_string()),
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
        Err("Cannot reset a live 5Paisa account. Use paper mode.".into())
    }

    fn mode(&self) -> TradingMode {
        TradingMode::Live
    }

    fn broker_name(&self) -> &str {
        "5Paisa"
    }
}

// ── Helper ───────────────────────────────────────────────────────────────────

pub fn create_fivepaisa_broker(
    app_key: &str,
    encry_key: &str,
    user_id: &str,
    client_code: &str,
) -> std::sync::Arc<dyn BrokerAdapter> {
    let access_token = std::env::var("FIVEPAISA_ACCESS_TOKEN")
        .ok()
        .unwrap_or_default();
    std::sync::Arc::new(FivePaisaBroker::new(
        app_key,
        encry_key,
        user_id,
        client_code,
        &access_token,
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broker_name_and_mode() {
        let broker = FivePaisaBroker::new("k", "e", "u", "c", "");
        assert_eq!(broker.broker_name(), "5Paisa");
        assert_eq!(broker.mode(), TradingMode::Live);
    }

    #[test]
    fn test_tp_order_type() {
        assert_eq!(FivePaisaBroker::tp_order_type(OrderType::Market), "Market");
        assert_eq!(FivePaisaBroker::tp_order_type(OrderType::Limit), "Limit");
        assert_eq!(
            FivePaisaBroker::tp_order_type(OrderType::StopLoss),
            "StopLoss"
        );
    }

    #[test]
    fn test_tp_transaction_type() {
        assert_eq!(
            FivePaisaBroker::tp_transaction_type(TradeDirection::Long),
            "Buy"
        );
        assert_eq!(
            FivePaisaBroker::tp_transaction_type(TradeDirection::Short),
            "Sell"
        );
    }

    #[test]
    fn test_parse_order_status() {
        assert_eq!(
            FivePaisaBroker::parse_order_status("COMPLETE"),
            OrderStatus::Filled
        );
        assert_eq!(
            FivePaisaBroker::parse_order_status("PENDING"),
            OrderStatus::Pending
        );
        assert_eq!(
            FivePaisaBroker::parse_order_status("REJECTED"),
            OrderStatus::Rejected {
                reason: "Order rejected by 5Paisa".into()
            }
        );
        assert_eq!(
            FivePaisaBroker::parse_order_status("CANCELLED"),
            OrderStatus::Cancelled
        );
    }

    #[test]
    fn test_exchange_code() {
        assert_eq!(FivePaisaBroker::exchange_code("RELIANCE"), "N");
        assert_eq!(FivePaisaBroker::exchange_code("NIFTY"), "N");
    }
}
