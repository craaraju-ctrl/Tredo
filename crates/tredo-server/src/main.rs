//! # tredo HTTP Server — Production Trading Backend
//!
//! Serves the frontend statically and provides REST API endpoints for
//! all trading operations. Routes through `BrokerRegistry` which dispatches
//! to either `PaperBroker` (virtual money) or a live broker adapter (real money).
//!
//! ## API Endpoints
//! - `GET  /api/summary`       — Portfolio summary (current mode)
//! - `GET  /api/positions`     — Open positions
//! - `GET  /api/trades`        — Recent trade history
//! - `POST /api/trade`         — Place an order
//! - `POST /api/close`         — Close a position
//! - `POST /api/price`         — Update market price for a symbol
//! - `POST /api/reset`         — Reset paper portfolio
//! - `GET  /api/mode`          — Get current mode (paper/live)
//! - `POST /api/mode`          — Switch mode
//! - `POST /api/broker/config` — Update broker API config
//! - `POST /api/broker/test`   — Test broker connection
//! - `WS   /ws`                — Real-time updates
//!
//! ## Run
//! ```bash
//! cargo run -p tredo-server -- --port 8080
//! ```

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tredo_core::paper_engine::*;
use tredo_core::TradeDirection;

// ── Application State ────────────────────────────────────────────────────────

struct AppState {
    registry: BrokerRegistry,
    price_tx: broadcast::Sender<PriceUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PriceUpdate {
    symbol: String,
    price: f64,
    timestamp: i64,
}

// ── API Response Types ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    success: bool,
    data: Option<T>,
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn err(msg: &str) -> Json<Self> {
        Json(Self {
            success: false,
            data: None,
            error: Some(msg.to_string()),
        })
    }
}

impl ApiResponse<()> {
    fn ok_empty() -> Json<Self> {
        Json(ApiResponse {
            success: true,
            data: None,
            error: None,
        })
    }
}

// ── Request Types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TradeRequest {
    symbol: String,
    direction: String,
    qty: i32,
    order_type: Option<String>,
    price: Option<f64>,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
    strategy: Option<String>,
}

#[derive(Deserialize)]
struct CloseRequest {
    position_id: String,
    exit_price: f64,
}

#[derive(Deserialize)]
struct PriceRequest {
    symbol: String,
    price: f64,
}

#[derive(Deserialize)]
struct ModeRequest {
    mode: String,
}

#[derive(Deserialize)]
struct BrokerConfigRequest {
    broker: String,
    api_key: String,
    api_secret: String,
    base_url: Option<String>,
}

// ── Routes ───────────────────────────────────────────────────────────────────

async fn handle_get_summary(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<PortfolioSummary>> {
    let broker = state.registry.active_broker().await;
    match broker.get_summary().await {
        Ok(s) => Json(ApiResponse::ok(s)),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        }),
    }
}

async fn handle_get_positions(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<Position>>> {
    let broker = state.registry.active_broker().await;
    match broker.get_positions().await {
        Ok(p) => Json(ApiResponse::ok(p)),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        }),
    }
}

async fn handle_get_trades(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<ClosedTrade>>> {
    let broker = state.registry.active_broker().await;
    match broker.get_recent_trades(50).await {
        Ok(t) => Json(ApiResponse::ok(t)),
        Err(e) => Json(ApiResponse {
            success: false,
            data: None,
            error: Some(e),
        }),
    }
}

async fn handle_place_trade(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TradeRequest>,
) -> Json<ApiResponse<String>> {
    let direction = match req.direction.to_lowercase().as_str() {
        "long" | "buy" => TradeDirection::Long,
        "short" | "sell" => TradeDirection::Short,
        _ => return ApiResponse::err("Invalid direction. Use 'long' or 'short'."),
    };

    let order_type = match req.order_type.as_deref().unwrap_or("market") {
        "market" => OrderType::Market,
        "limit" => OrderType::Limit,
        _ => return ApiResponse::err("Invalid order type. Use 'market' or 'limit'."),
    };

    let request = OrderRequest {
        symbol: req.symbol,
        direction,
        order_type,
        qty: req.qty,
        price: req.price,
        stop_loss: req.stop_loss,
        take_profit: req.take_profit,
        strategy: req.strategy,
        client_order_id: None,
    };

    // Get market price (use provided price or request price)
    let market_price = req.price.unwrap_or(0.0);
    if market_price <= 0.0 {
        return ApiResponse::err("Price must be provided and positive.");
    }

    let broker = state.registry.active_broker().await;
    match broker.place_order(request, market_price).await {
        Ok(id) => Json(ApiResponse::ok(id)),
        Err(e) => ApiResponse::err(&e),
    }
}

async fn handle_close_position(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CloseRequest>,
) -> Json<ApiResponse<ClosedTrade>> {
    let broker = state.registry.active_broker().await;
    match broker
        .close_position(&req.position_id, req.exit_price)
        .await
    {
        Ok(t) => Json(ApiResponse::ok(t)),
        Err(e) => ApiResponse::err(&e),
    }
}

async fn handle_update_price(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PriceRequest>,
) -> Json<ApiResponse<Vec<ClosedTrade>>> {
    let broker = state.registry.active_broker().await;
    match broker.update_price(&req.symbol, req.price).await {
        Ok(closed) => {
            // Broadcast price update
            let _ = state.price_tx.send(PriceUpdate {
                symbol: req.symbol.clone(),
                price: req.price,
                timestamp: chrono::Utc::now().timestamp_millis(),
            });
            Json(ApiResponse::ok(closed))
        }
        Err(e) => ApiResponse::err(&e),
    }
}

async fn handle_reset(State(state): State<Arc<AppState>>) -> Json<ApiResponse<()>> {
    let broker = state.registry.active_broker().await;
    match broker.reset().await {
        Ok(()) => ApiResponse::ok_empty(),
        Err(e) => ApiResponse::err(&e),
    }
}

async fn handle_get_mode(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mode = state.registry.current_mode().await;
    let name = state.registry.current_broker_name().await;
    Json(ApiResponse::ok(serde_json::json!({
        "mode": mode.to_string(),
        "broker": name,
    })))
}

async fn handle_set_mode(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ModeRequest>,
) -> Json<ApiResponse<String>> {
    let mode = match req.mode.to_lowercase().as_str() {
        "paper" => TradingMode::Paper,
        "live" => TradingMode::Live,
        _ => return ApiResponse::err("Invalid mode. Use 'paper' or 'live'."),
    };
    match state.registry.set_mode(mode).await {
        Ok(()) => Json(ApiResponse::ok(mode.to_string())),
        Err(e) => ApiResponse::err(&e),
    }
}

async fn handle_broker_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BrokerConfigRequest>,
) -> Json<ApiResponse<String>> {
    match req.broker.to_lowercase().as_str() {
        "zerodha" | "kite" => {
            let base_url = req
                .base_url
                .unwrap_or_else(|| "https://api.kite.trade".to_string());
            let broker = Arc::new(ZerodhaKiteBroker::new(
                &req.api_key,
                &req.api_secret,
                &base_url,
            ));
            state.registry.register_live_broker(broker).await;
            Json(ApiResponse::ok(
                "Zerodha Kite broker registered".to_string(),
            ))
        }
        "angel" => {
            let broker = Arc::new(AngelOneBroker::new(&req.api_key, &req.api_secret));
            state.registry.register_live_broker(broker).await;
            Json(ApiResponse::ok("Angel One broker registered".to_string()))
        }
        _ => ApiResponse::err(&format!("Unknown broker: {}", req.broker)),
    }
}

async fn handle_broker_test(State(state): State<Arc<AppState>>) -> Json<ApiResponse<String>> {
    let broker = state.registry.active_broker().await;
    match broker.connect().await {
        Ok(()) => Json(ApiResponse::ok(format!(
            "Connected to {}",
            broker.broker_name()
        ))),
        Err(e) => ApiResponse::err(&e),
    }
}

async fn handle_get_status(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<serde_json::Value>> {
    let broker = state.registry.active_broker().await;
    let mode = state.registry.current_mode().await;
    let summary = broker.get_summary().await.ok();
    let positions = broker.get_positions().await.unwrap_or_default();

    Json(ApiResponse::ok(serde_json::json!({
        "mode": mode.to_string(),
        "broker": broker.broker_name(),
        "summary": summary,
        "open_positions": positions.len(),
        "trading_enabled": true,
    })))
}

// ── WebSocket ────────────────────────────────────────────────────────────────

async fn handle_ws(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_socket(socket, state))
}

async fn handle_ws_socket(socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.price_tx.subscribe();
    let (mut sender, _receiver) = socket.split();

    // Send initial state
    let broker = state.registry.active_broker().await;
    let initial_state = serde_json::json!({
        "type": "initial_state",
        "mode": state.registry.current_mode().await.to_string(),
        "summary": broker.get_summary().await.ok().unwrap_or_default(),
        "positions": broker.get_positions().await.unwrap_or_default(),
    });
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&initial_state).unwrap(),
        ))
        .await;

    // Stream price updates
    while let Ok(update) = rx.recv().await {
        let msg = serde_json::json!({
            "type": "price_update",
            "symbol": update.symbol,
            "price": update.price,
            "timestamp": update.timestamp,
        });
        if sender
            .send(Message::Text(serde_json::to_string(&msg).unwrap()))
            .await
            .is_err()
        {
            break;
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));
    log::info!("[tredo Server] Starting production trading backend...");

    // Initialize paper engine with default config
    let config = PaperEngineConfig::default();
    let registry = BrokerRegistry::new(config);

    // Connect paper broker by default
    registry
        .set_mode(TradingMode::Paper)
        .await
        .expect("Failed to initialize paper broker");

    // Price update broadcast channel
    let (price_tx, _) = broadcast::channel::<PriceUpdate>(256);

    let state = Arc::new(AppState { registry, price_tx });

    // Build router
    let app = Router::new()
        // Serve static frontend files
        .route_service(
            "/",
            get(|| async { axum::response::Redirect::to("/index.html") }),
        )
        .nest_service("/", ServeDir::new("src-tauri/frontend"))
        // API routes
        .route("/api/summary", get(handle_get_summary))
        .route("/api/positions", get(handle_get_positions))
        .route("/api/trades", get(handle_get_trades))
        .route("/api/trade", post(handle_place_trade))
        .route("/api/close", post(handle_close_position))
        .route("/api/price", post(handle_update_price))
        .route("/api/reset", post(handle_reset))
        .route("/api/mode", get(handle_get_mode).post(handle_set_mode))
        .route("/api/status", get(handle_get_status))
        .route("/api/broker/config", post(handle_broker_config))
        .route("/api/broker/test", post(handle_broker_test))
        .route("/ws", get(handle_ws))
        // CORS for development
        .layer(CorsLayer::permissive())
        // Shared state
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    log::info!("[tredo Server] Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app).await.expect("Server failed");
}
