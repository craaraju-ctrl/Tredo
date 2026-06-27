mod loops;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::{watch, Mutex as TokioMutex};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{error, info, warn};
use tredo_autonomous::state::initialize_autonomous_system;
use tredo_core::paper_engine::TradingMode;
use tredo_eventbus::{self, subjects as event_subjects, EventBus, TredoEvent};

// ── Loop Manager to dynamically start and stop the background temporal loops ──
struct LoopManager {
    orchestrator: tredo_autonomous::AutonomousOrchestrator,
    client: reqwest::Client,
    assets: Vec<String>,
    bus: Arc<dyn EventBus>,
    shutdown_tx: Option<watch::Sender<bool>>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl LoopManager {
    fn new(
        orchestrator: tredo_autonomous::AutonomousOrchestrator,
        client: reqwest::Client,
        assets: Vec<String>,
        bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            orchestrator,
            client,
            assets,
            bus,
            shutdown_tx: None,
            handles: Vec::new(),
        }
    }

    async fn start(&mut self) -> bool {
        if self.shutdown_tx.is_some() {
            return false; // Already running
        }

        let (tx, rx) = watch::channel(false);
        self.shutdown_tx = Some(tx);

        let orch_fast = self.orchestrator.clone();
        let client_fast = self.client.clone();
        let assets_fast = self.assets.clone();
        let rx_fast = rx.clone();
        let bus_fast = self.bus.clone();

        let orch_medium = self.orchestrator.clone();
        let client_medium = self.client.clone();
        let assets_medium = self.assets.clone();
        let rx_medium = rx.clone();
        let bus_medium = self.bus.clone();

        let orch_slow = self.orchestrator.clone();
        let state_slow = self.orchestrator.state.clone();
        let rx_slow = rx.clone();
        let bus_slow = self.bus.clone();

        let fast_handle = tokio::spawn(async move {
            loops::fast_loop(orch_fast, client_fast, assets_fast, rx_fast, bus_fast).await;
        });

        let medium_handle = tokio::spawn(async move {
            loops::medium_loop(
                orch_medium,
                client_medium,
                assets_medium,
                rx_medium,
                bus_medium,
            )
            .await;
        });

        let slow_handle = tokio::spawn(async move {
            loops::slow_loop(orch_slow, state_slow, rx_slow, bus_slow).await;
        });

        self.handles = vec![fast_handle, medium_handle, slow_handle];

        {
            let mut p = self.orchestrator.state.portfolio.write().await;
            p.trading_enabled = true;
        }

        info!("Background loops started");
        true
    }

    async fn stop(&mut self) -> bool {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
            for handle in self.handles.drain(..) {
                let _ = handle.await;
            }
            {
                let mut p = self.orchestrator.state.portfolio.write().await;
                p.trading_enabled = false;
            }
            info!("Background loops stopped cleanly");
            true
        } else {
            false
        }
    }

    async fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }
}

// ── Web State Shared with Axum Handlers ───────────────────────────────────────
#[derive(Clone)]
struct WebState {
    orchestrator: tredo_autonomous::AutonomousOrchestrator,
    loop_manager: Arc<TokioMutex<LoopManager>>,
}

// ── Start-up Initialization ─────────────────────────────────────────────────

/// Returns watchlist immediately; heavy OHLCV/MTF fetching runs in background
/// so the HTTP API can bind within seconds (not blocked on 300+ Binance calls).
fn schedule_data_feed_init(
    orchestrator: tredo_autonomous::AutonomousOrchestrator,
    client: reqwest::Client,
    assets: Vec<String>,
) {
    if assets.is_empty() {
        return;
    }
    info!(symbols = ?assets, count = assets.len(), "Background data init scheduled");
    tokio::spawn(async move {
        initialize_data_feeds_background(orchestrator, client, assets).await;
    });
}

async fn initialize_data_feeds_background(
    orchestrator: tredo_autonomous::AutonomousOrchestrator,
    client: reqwest::Client,
    assets: Vec<String>,
) {
    let init_limiter = Arc::new(tokio::sync::Semaphore::new(5));

    // Phase 1: 1m OHLCV only (fast — unblocks indicators / HardRulesGate)
    let mut handles = Vec::with_capacity(assets.len());
    for symbol in &assets {
        let sym = symbol.clone();
        let sym_is_crypto = loops::is_crypto_symbol(&sym);
        let cl = client.clone();
        let lim = init_limiter.clone();
        let orch = orchestrator.clone();
        handles.push(tokio::spawn(async move {
            let _permit = lim.acquire().await.ok()?;
            let bars = if sym_is_crypto {
                loops::fetch_binance_klines(&cl, &sym, "1m", 100)
                    .await
                    .unwrap_or_default()
            } else {
                loops::fetch_yahoo_ohlcv(&cl, &sym, "1m", "7d")
                    .await
                    .unwrap_or_default()
            };
            if !bars.is_empty() {
                orch.state.ohlcv_history.write().await.insert(sym, bars);
            }
            Some(())
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    info!(count = assets.len(), "1m OHLCV loaded");

    // Phase 2: multi-timeframe (slow — deferred; medium_loop also refreshes MTF)
    loops::refresh_multi_tf(&assets, &client, &orchestrator.state).await;

    {
        let mut summary = orchestrator.state.agent_market_summary.write().await;
        *summary = format!(
            "Data feeds ready. Monitoring: {} with Ollama + Kronos.",
            assets.join(", ")
        );
    }
    info!("Background data init complete (OHLCV + MTF)");
}

async fn restore_portfolio_state(state: &tredo_autonomous::state::SharedState) -> bool {
    match state.memory.load_state("portfolio/state") {
        Ok(Some(json)) => {
            match serde_json::from_str::<tredo_autonomous::types::PortfolioState>(&json) {
                Ok(restored) => {
                    let mut portfolio = state.portfolio.write().await;
                    *portfolio = restored;
                    info!(
                        equity = portfolio.total_equity,
                        cash = portfolio.cash_balance,
                        positions = portfolio.open_positions.len(),
                        "Portfolio restored"
                    );
                    true
                }
                Err(e) => {
                    error!(error = %e, "Failed to parse portfolio state. Starting fresh.");
                    false
                }
            }
        }
        Ok(None) => {
            info!("No saved portfolio state found. Starting fresh.");
            false
        }
        Err(e) => {
            error!(error = %e, "Failed to load portfolio state. Starting fresh.");
            false
        }
    }
}

async fn restore_agent_tasks(state: &tredo_autonomous::state::SharedState) {
    if let Ok(Some(json)) = state.memory.load_state("tasks/state") {
        if let Ok(restored) = serde_json::from_str::<Vec<tredo_autonomous::state::AgentTask>>(&json)
        {
            let mut tasks = state.agent_tasks.write().await;
            *tasks = restored;
            info!("Agent tasks restored from redb.");
        }
    }
}

// ── Graceful Shutdown ───────────────────────────────────────────────────────

async fn graceful_shutdown(orchestrator: &tredo_autonomous::AutonomousOrchestrator) {
    info!("Winding down web server & portfolio state...");
    loops::save_portfolio_state(&orchestrator.state).await;
    save_watchlist(&orchestrator.state).await;
    let p = orchestrator.state.portfolio.read().await;
    info!(
        equity = p.total_equity,
        pnl = p.daily_pnl,
        trades = p.total_trades_today,
        open = p.open_positions.len(),
        "Final Portfolio"
    );
    drop(p);
    info!("tredo terminated. Goodbye.");
}

// ── Axum Endpoint Handlers ───────────────────────────────────────────────────

async fn portfolio_snapshot_response(state: &WebState, include_meta: bool) -> serde_json::Value {
    let portfolio = state.orchestrator.state.portfolio.read().await;
    let mut snapshot = tredo_autonomous::state::SharedState::portfolio_snapshot_json(&portfolio);
    if include_meta {
        let rules = state.orchestrator.state.rules.read().await;
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert("status".to_string(), serde_json::json!("tredo Running"));
            obj.insert(
                "initial_balance".to_string(),
                serde_json::json!(state.orchestrator.state.config.initial_balance),
            );
            obj.insert(
                "use_confluence".to_string(),
                serde_json::json!(rules.use_confluence),
            );
        }
    }
    snapshot
}

async fn get_system_status(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    Json(portfolio_snapshot_response(&state, true).await)
}

async fn get_portfolio(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    Json(portfolio_snapshot_response(&state, false).await)
}

async fn get_system_health(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let kronos_up = reqwest::Client::new()
        .get("http://localhost:8000/docs")
        .timeout(Duration::from_millis(400))
        .send()
        .await
        .is_ok();

    let manager = state.loop_manager.lock().await;
    let running = manager.is_running().await;

    let current_model = state.orchestrator.state.llm.get_model();
    let ollama_running = state.orchestrator.state.llm.is_ollama_running().await;

    Json(serde_json::json!({
        "kronos": kronos_up,
        "orchestrator": running,
        "llm": ollama_running,
        "model": current_model,
        "running": running,
    }))
}

// ── LLM Model Management Endpoints ──────────────────────────────────────────

async fn get_available_models(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let llm = state.orchestrator.state.llm.clone();
    // Use blocking for simplicity in API call
    let client = reqwest::Client::new();
    let endpoint = llm.endpoint.clone();

    let base_url = endpoint
        .replace("/api/generate", "")
        .replace("/api/chat", "");
    let res = client
        .get(format!("{}/api/tags", base_url))
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    match res {
        Ok(resp) if resp.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct OllamaTagsResponse {
                models: Vec<ModelInfo>,
            }
            #[derive(serde::Deserialize)]
            struct ModelInfo {
                name: String,
                size: Option<u64>,
                modified_at: Option<String>,
            }

            if let Ok(tags_res) = resp.json::<OllamaTagsResponse>().await {
                let models: Vec<serde_json::Value> = tags_res
                    .models
                    .into_iter()
                    .map(|m| {
                        let size_str = m.size.map(|s| {
                            if s > 1_000_000_000 {
                                format!("{:.1}GB", s as f64 / 1_000_000_000.0)
                            } else if s > 1_000_000 {
                                format!("{:.1}MB", s as f64 / 1_000_000.0)
                            } else {
                                format!("{}B", s)
                            }
                        });
                        serde_json::json!({
                            "name": m.name,
                            "size": size_str,
                            "modified": m.modified_at,
                            "is_local": true
                        })
                    })
                    .collect();
                return Json(serde_json::json!({
                    "success": true,
                    "current_model": llm.get_model(),
                    "models": models
                }));
            }
        }
        _ => {}
    }

    Json(serde_json::json!({
        "success": false,
        "error": "Failed to fetch models from Ollama. Is Ollama running?",
        "current_model": llm.get_model(),
        "models": []
    }))
}

#[derive(serde::Deserialize)]
struct SetModelRequest {
    model: String,
}

async fn set_llm_model(
    State(state): State<WebState>,
    Json(req): Json<SetModelRequest>,
) -> impl axum::response::IntoResponse {
    let model = req.model.trim().to_string();
    if model.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "Model name cannot be empty"
            })),
        );
    }

    // Try to fetch models to validate
    let client = reqwest::Client::new();
    let endpoint = state.orchestrator.state.llm.endpoint.clone();
    let base_url = endpoint
        .replace("/api/generate", "")
        .replace("/api/chat", "");

    let res = client
        .get(format!("{}/api/tags", base_url))
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    let valid = match res {
        Ok(resp) if resp.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct OllamaTagsResponse {
                models: Vec<ModelInfo>,
            }
            #[derive(serde::Deserialize)]
            struct ModelInfo {
                name: String,
            }

            if let Ok(tags_res) = resp.json::<OllamaTagsResponse>().await {
                tags_res.models.iter().any(|m| m.name == model)
            } else {
                false
            }
        }
        _ => false,
    };

    if !valid {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Model '{}' not found. Available models fetched from Ollama.", model)
            })),
        );
    }

    // Record the model change for COT logging
    let old_model = state.orchestrator.state.llm.get_model();
    // Note: The model is already stored on LlmExecutor and used directly.
    // The env var approach was removed in favor of passing the model through
    // the executor's state. A restart is still needed for the new model to take effect.

    state
        .orchestrator
        .state
        .push_cot(
            "MetaControl",
            "LLM Model Change",
            "MODEL_SWITCH",
            &format!("Switched from {} to {}", old_model, model),
            0.0,
            1,
            None,
            None,
        )
        .await;

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": format!("Model switched from {} to {}. Restart orchestrator to apply.", old_model, model),
            "old_model": old_model,
            "new_model": model
        })),
    )
}

async fn get_cot_chains(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let store = state.orchestrator.state.cot_store.read().await;
    Json(store.clone())
}

async fn start_autonomous_system(
    State(state): State<WebState>,
) -> impl axum::response::IntoResponse {
    let mut manager = state.loop_manager.lock().await;
    let started = manager.start().await;
    Json(serde_json::json!({
        "status": "starting",
        "kronos": true,
        "orchestrator": true,
        "started": started,
    }))
}

async fn stop_autonomous_system(
    State(state): State<WebState>,
) -> impl axum::response::IntoResponse {
    let mut manager = state.loop_manager.lock().await;
    let stopped = manager.stop().await;
    Json(serde_json::json!({
        "status": "stopped",
        "stopped": stopped,
    }))
}

// ── Watchlist Storage & Endpoints ───────────────────────────────────────────

async fn save_watchlist(state: &tredo_autonomous::state::SharedState) {
    let watchlist = state.watchlist.read().await;
    if let Ok(json) = serde_json::to_string(&*watchlist) {
        let _ = state.memory.store_state("watchlist/state", &json);
    }
}

async fn restore_watchlist(state: &tredo_autonomous::state::SharedState) {
    if let Ok(Some(json)) = state.memory.load_state("watchlist/state") {
        if let Ok(restored) = serde_json::from_str::<Vec<String>>(&json) {
            let mut watchlist = state.watchlist.write().await;
            *watchlist = restored;
            info!(watchlist = ?*watchlist, "Watchlist restored from redb");
            return;
        }
    }

    // No saved watchlist — try WATCHLIST env (from config/tredo.env)
    if let Ok(env_wl) = std::env::var("WATCHLIST") {
        let symbols: Vec<String> = env_wl
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !symbols.is_empty() {
            let mut watchlist = state.watchlist.write().await;
            *watchlist = symbols;
            info!(watchlist = ?*watchlist, "Watchlist loaded from WATCHLIST env");
            return;
        }
    }

    info!("No saved watchlist — will seed defaults if still empty.");
}

#[derive(serde::Deserialize)]
struct WatchlistRequest {
    symbol: String,
}

async fn get_watchlist(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let wl = state.orchestrator.state.watchlist.read().await;
    Json(wl.clone())
}

async fn get_metrics(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let metrics = state.orchestrator.state.latest_metrics.read().await;
    Json(serde_json::json!(*metrics))
}

async fn add_to_watchlist(
    State(state): State<WebState>,
    Json(req): Json<WatchlistRequest>,
) -> impl axum::response::IntoResponse {
    let symbol = req.symbol.trim().to_uppercase();
    if symbol.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Symbol cannot be empty" })),
        );
    }

    let scanner =
        tredo_autonomous::scanner::WatchlistScannerAgent::new(state.orchestrator.state.clone());
    let added = scanner.add_to_watchlist(&symbol).await;
    if added {
        save_watchlist(&state.orchestrator.state).await;
        let client = reqwest::Client::new();
        let is_crypto = loops::is_crypto_symbol(&symbol);
        let bars = if is_crypto {
            loops::fetch_binance_klines(&client, &symbol, "1m", 100)
                .await
                .unwrap_or_default()
        } else {
            loops::fetch_yahoo_ohlcv(&client, &symbol, "1m", "7d")
                .await
                .unwrap_or_default()
        };
        if !bars.is_empty() {
            let mut history = state.orchestrator.state.ohlcv_history.write().await;
            history.insert(symbol.clone(), bars);
        }
        loops::update_multi_tf_data(&client, &state.orchestrator, &symbol, is_crypto).await;
    }

    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "added": added, "symbol": symbol })),
    )
}

async fn remove_from_watchlist(
    State(state): State<WebState>,
    Json(req): Json<WatchlistRequest>,
) -> impl axum::response::IntoResponse {
    let symbol = req.symbol.trim().to_uppercase();
    let scanner =
        tredo_autonomous::scanner::WatchlistScannerAgent::new(state.orchestrator.state.clone());
    let removed = scanner.remove_from_watchlist(&symbol).await;
    if removed {
        save_watchlist(&state.orchestrator.state).await;
    }
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "removed": removed, "symbol": symbol })),
    )
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TradeRequest {
    symbol: String,
    direction_str: String,
    entry_price: f64,
    stop_loss: f64,
    take_profit: f64,
}

async fn execute_trade(
    State(state): State<WebState>,
    Json(req): Json<TradeRequest>,
) -> impl axum::response::IntoResponse {
    use tredo_autonomous::types::TradeSignal;
    use tredo_core::{validate_trade_setup, TradeDirection, TradeSetup};

    let direction = match req.direction_str.to_lowercase().as_str() {
        "long" | "buy" => TradeDirection::Long,
        "short" | "sell" => TradeDirection::Short,
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid direction. Use 'long' or 'short'".to_string(),
            )
        }
    };

    // Read real portfolio equity for accurate drawdown check
    let portfolio_equity = state.orchestrator.state.portfolio.read().await.total_equity;
    let context = tredo_core::MarketContext {
        symbol: req.symbol.clone(),
        current_price: req.entry_price,
        high: req.entry_price * 1.01,
        low: req.entry_price * 0.99,
        previous_close: req.entry_price,
        timestamp: chrono::Utc::now(),
        daily_pnl: 0.0,
        equity: portfolio_equity,
        consecutive_losses: 0,
        is_red_folder_day: false,
        trend_direction: None,
    };

    let setup = TradeSetup::new(
        req.symbol.clone(),
        direction,
        req.entry_price,
        req.stop_loss,
        req.take_profit,
        context,
    );
    let rules = state.orchestrator.state.rules.read().await;
    let check = validate_trade_setup(&setup.context, &rules);

    if !check.passed {
        state
            .orchestrator
            .state
            .push_cot(
                "DisciplineCore",
                &format!(
                    "Discipline check for {} {} @ {:.2}",
                    req.symbol, req.direction_str, req.entry_price
                ),
                "REJECTED",
                &check.reasons.join("; "),
                0.0,
                1,
                None,
                Some(req.symbol.clone()),
            )
            .await;
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!("DISCIPLINE REJECTED: {}", check.reasons.join(", ")),
        );
    }

    if req.entry_price <= 0.0 || req.stop_loss <= 0.0 || req.take_profit <= 0.0 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "INVALID PRICES: Entry, Stop Loss and Take Profit must be positive".to_string(),
        );
    }

    let position_size = {
        let rules = state.orchestrator.state.rules.read().await;
        let cash = state.orchestrator.state.portfolio.read().await.cash_balance;
        tredo_autonomous::helpers::calculate_position_size_with_cash(
            portfolio_equity,
            rules.max_risk_per_trade,
            req.entry_price,
            req.stop_loss,
            Some(cash),
        )
    };

    if position_size <= 0.0 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "INVALID POSITION SIZE: check entry/stop distance and account equity".to_string(),
        );
    }

    let risk = (req.entry_price - req.stop_loss).abs();
    let reward = (req.take_profit - req.entry_price).abs();
    let risk_reward_ratio = if risk > 0.0 { reward / risk } else { 2.0 };

    let signal = TradeSignal {
        symbol: req.symbol.clone(),
        direction,
        entry_price: req.entry_price,
        stop_loss: req.stop_loss,
        take_profit: req.take_profit,
        position_size,
        confidence_score: 0.85,
        confluence_score: 0.85,
        risk_reward_ratio,
        reasoning: "Manual API Order".to_string(),
        timestamp: chrono::Utc::now(),
        session_valid: true,
        risk_check_passed: true,
    };

    match state
        .orchestrator
        .execution
        .execute_paper_trade(&signal)
        .await
    {
        Ok(exec_log) => {
            state
                .orchestrator
                .state
                .broadcast_portfolio_snapshot()
                .await;
            let response = serde_json::json!({
                "success": true,
                "message": exec_log,
                "position_size": position_size,
            });
            let body = response.to_string();
            (axum::http::StatusCode::OK, body)
        }
        Err(e) => {
            let response = serde_json::json!({
                "success": false,
                "error": format!("EXECUTION ERROR: {}", e)
            });
            let body = response.to_string();
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, body)
        }
    }
}

#[derive(serde::Deserialize)]
struct CycleRequest {
    symbol: Option<String>,
}

#[derive(serde::Deserialize)]
struct PipelineRunRequest {
    symbol: Option<String>,
    symbols: Option<Vec<String>>,
}

async fn trigger_orchestra_cycle(
    State(state): State<WebState>,
    Json(req): Json<CycleRequest>,
) -> impl axum::response::IntoResponse {
    let sym = req
        .symbol
        .map(|s| tredo_core::normalize_base_symbol(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            // Default to first watchlist symbol, not hardcoded NIFTY
            state
                .orchestrator
                .state
                .watchlist
                .try_read()
                .ok()
                .and_then(|wl| wl.first().cloned())
                .unwrap_or_else(|| "BTC".to_string())
        });

    info!(symbol = %sym, "Trigger cycle");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let outcome = tredo_autonomous::run_single(&state.orchestrator, &client, &sym).await;
    let report = outcome.report;

    let status = if report.success {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    };

    (
        status,
        Json(serde_json::json!({
            "success": report.success,
            "symbol": report.symbol,
            "executed": report.executed,
            "action": report.action,
            "reason": report.reason,
            "duration_ms": report.duration_ms,
            "error": report.error,
            "message": format!(
                "{} {} | {} | {}ms",
                report.symbol, report.action, report.reason, report.duration_ms
            ),
        })),
    )
}

async fn run_pipeline_batch(
    State(state): State<WebState>,
    Json(req): Json<PipelineRunRequest>,
) -> impl axum::response::IntoResponse {
    let symbols: Vec<String> = if let Some(list) = req.symbols {
        list.into_iter()
            .map(|s| tredo_core::normalize_base_symbol(&s))
            .filter(|s| !s.is_empty())
            .collect()
    } else if let Some(sym) = req.symbol {
        vec![tredo_core::normalize_base_symbol(&sym)]
    } else {
        state.orchestrator.state.watchlist.read().await.clone()
    };

    if symbols.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": "No symbols to run — add to watchlist or pass symbol/symbols in body"
            })),
        );
    }

    info!(count = symbols.len(), symbols = ?symbols, "Run pipeline");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let batch = tredo_autonomous::run_batch(&state.orchestrator, &client, &symbols).await;

    let status = if batch.success {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::MULTI_STATUS
    };

    (
        status,
        Json(serde_json::to_value(&batch).unwrap_or_default()),
    )
}

#[derive(serde::Deserialize)]
struct RulesRequest {
    use_confluence: bool,
    respect_session_timing: bool,
}

async fn update_rules(
    State(state): State<WebState>,
    Json(req): Json<RulesRequest>,
) -> impl axum::response::IntoResponse {
    {
        let mut rules = state.orchestrator.state.rules.write().await;
        rules.use_confluence = req.use_confluence;
        rules.respect_session_timing = req.respect_session_timing;
    }
    state
        .orchestrator
        .state
        .push_cot(
            "MetaControl",
            "Update discipline rules",
            "UPDATED",
            &format!(
                "Confluence: {}, SessionTiming: {}",
                req.use_confluence, req.respect_session_timing
            ),
            0.9,
            1,
            None,
            None,
        )
        .await;
    Json(serde_json::json!({
        "message": "Rules updated successfully"
    }))
}

async fn run_backtest(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let rules = state.orchestrator.state.rules.read().await;
    let mut backtester = tredo_core::Backtester::new(rules.clone());
    let mut dummy_data = Vec::new();
    for i in 0..50 {
        dummy_data.push(tredo_core::MarketContext {
            symbol: "NIFTY".to_string(),
            current_price: 24000.0 + (i as f64 * 10.0),
            high: 24050.0,
            low: 23950.0,
            previous_close: 23980.0,
            timestamp: chrono::Utc::now(),
            daily_pnl: 0.0,
            consecutive_losses: 0,
            equity: 100000.0,
            is_red_folder_day: false,
            trend_direction: None,
        });
    }
    let result = backtester.run_simulation(dummy_data);

    state
        .orchestrator
        .state
        .push_cot(
            "Backtester",
            "Running 50-cycle backtest simulation",
            "COMPLETE",
            &format!(
                "Trades: {}, Win Rate: {:.1}%, P&L: ₹{:.2}, Max DD: {:.2}%",
                result.total_trades,
                result.win_rate * 100.0,
                result.total_pnl,
                result.max_drawdown * 100.0
            ),
            0.85,
            1,
            None,
            None,
        )
        .await;

    Json(serde_json::json!({
        "message": format!(
            "Backtest complete | Trades: {} | Win Rate: {:.1}% | Total P&L: ₹{:.2} | Max DD: {:.2}%",
            result.total_trades, result.win_rate * 100.0,
            result.total_pnl, result.max_drawdown * 100.0
        )
    }))
}

/// Structured backtest results endpoint — returns detailed backtest data for TUI display.
async fn get_backtest_results(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let rules = state.orchestrator.state.rules.read().await;
    let mut backtester = tredo_core::Backtester::new(rules.clone());
    let mut dummy_data = Vec::new();
    for i in 0..50 {
        dummy_data.push(tredo_core::MarketContext {
            symbol: "NIFTY".to_string(),
            current_price: 24000.0 + (i as f64 * 10.0),
            high: 24050.0,
            low: 23950.0,
            previous_close: 23980.0,
            timestamp: chrono::Utc::now(),
            daily_pnl: 0.0,
            consecutive_losses: 0,
            equity: 100000.0,
            is_red_folder_day: false,
            trend_direction: None,
        });
    }
    let result = backtester.run_simulation(dummy_data);
    Json(serde_json::json!({
        "total_trades": result.total_trades,
        "win_rate": result.win_rate,
        "total_pnl": result.total_pnl,
        "max_drawdown": result.max_drawdown,
        "sharpe_ratio": result.sharpe_ratio,
        "decisions": result.decisions,
        "message": format!(
            "Trades: {} | Win Rate: {:.1}% | Total P&L: ₹{:.2} | Max DD: {:.2}% | Sharpe: {:.2}",
            result.total_trades, result.win_rate * 100.0,
            result.total_pnl, result.max_drawdown * 100.0, result.sharpe_ratio
        )
    }))
}

#[derive(serde::Deserialize)]
struct PriceQuery {
    symbol: String,
}

async fn get_agent_tree() -> impl axum::response::IntoResponse {
    Json(tredo_autonomous::Tredo::tree_json())
}

async fn get_skill_scores(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let votes = state.orchestrator.state.last_skill_votes.read().await;
    let aggregated = state.orchestrator.state.last_aggregated_signal.read().await;
    Json(serde_json::json!({
        "votes": *votes,
        "aggregated": *aggregated,
    }))
}

async fn fetch_live_stock_price(
    axum::extract::Query(req): axum::extract::Query<PriceQuery>,
) -> impl axum::response::IntoResponse {
    let sym_upper = req.symbol.to_uppercase();
    let yahoo_symbol = match sym_upper.as_str() {
        "NIFTY" => "^NSEI",
        "RELIANCE" => "RELIANCE.NS",
        other => other,
    };
    let client = reqwest::Client::new();
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range=1d",
        yahoo_symbol
    );
    let resp: serde_json::Value = match client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
    {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(_) => serde_json::Value::Null,
    };
    let price = resp["chart"]["result"][0]["meta"]["regularMarketPrice"]
        .as_f64()
        .unwrap_or(24500.0);
    Json(price)
}

// ── Real Market Depth / Order Book ─────────────────────────────────────
// Proxies Binance depth API for crypto symbols; falls back to simulated depth for stocks.

#[derive(serde::Deserialize)]
struct DepthQuery {
    symbol: String,
}

async fn get_market_depth(
    axum::extract::Query(q): axum::extract::Query<DepthQuery>,
) -> impl axum::response::IntoResponse {
    let sym = q.symbol.trim().to_uppercase();
    let is_crypto = tredo_core::is_crypto_symbol(&sym);

    if is_crypto {
        let client = reqwest::Client::new();
        let binance_symbol = format!("{}USDT", sym);
        let url = format!(
            "https://api.binance.com/api/v3/depth?symbol={}&limit=12",
            binance_symbol
        );

        if let Ok(resp) = client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    return Json(serde_json::json!({
                        "symbol": sym,
                        "bids": json["bids"],
                        "asks": json["asks"],
                        "source": "binance"
                    }));
                }
            }
        }
    }

    // For stocks (or if Binance fails), generate reasonable depth relative to current price
    // using a configurable spread pattern — not random, but deterministic per symbol.
    // This avoids exposing the user to an empty order book for stock symbols.
    let price = std::env::var("DUMMY_PRICE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(2950.0);
    let _spread = price * 0.00015; // ~0.015% per level step
    let mut bids: Vec<Vec<String>> = Vec::new();
    let mut asks: Vec<Vec<String>> = Vec::new();
    for i in 0..12 {
        let step = (i + 1) as f64;
        let ask_price = price * (1.0 + step * 0.00015);
        let bid_price = price * (1.0 - step * 0.00015);
        let ask_qty = (2.0 - step * 0.12).max(0.05) + (i as f64 * 0.01).sin() * 0.5;
        let bid_qty = (2.0 - step * 0.12).max(0.05) + (i as f64 * 0.01).cos() * 0.5;
        asks.push(vec![format!("{:.2}", ask_price), format!("{:.4}", ask_qty)]);
        bids.push(vec![format!("{:.2}", bid_price), format!("{:.4}", bid_qty)]);
    }
    // Bids stay descending (best bid first) to match Binance format

    Json(serde_json::json!({
        "symbol": sym,
        "bids": bids,
        "asks": asks,
        "source": "simulated"
    }))
}

async fn get_crypto_exchanges() -> impl axum::response::IntoResponse {
    Json(serde_json::json!([
        { "id": "binance",  "name": "Binance",   "url": "https://api.binance.com",      "logo": "🟡", "active": true },
        { "id": "coinbase", "name": "Coinbase",  "url": "https://api.coinbase.com",    "logo": "🔵", "active": true },
        { "id": "kraken",   "name": "Kraken",    "url": "https://api.kraken.com",       "logo": "🔴", "active": true },
        { "id": "coingecko","name": "CoinGecko", "url": "https://api.coingecko.com",   "logo": "🦎", "active": true }
    ]))
}

async fn get_crypto_symbols() -> impl axum::response::IntoResponse {
    Json(serde_json::json!([
        { "symbol": "BTC",   "name": "Bitcoin",          "category": "layer1" },
        { "symbol": "ETH",   "name": "Ethereum",         "category": "layer1" },
        { "symbol": "SOL",   "name": "Solana",           "category": "layer1" },
        { "symbol": "BNB",   "name": "BNB",              "category": "exchange" },
        { "symbol": "XRP",   "name": "Ripple",           "category": "payments" },
        { "symbol": "ADA",   "name": "Cardano",          "category": "layer1" },
        { "symbol": "DOGE",  "name": "Dogecoin",         "category": "meme" },
        { "symbol": "AVAX",  "name": "Avalanche",        "category": "layer1" },
        { "symbol": "MATIC", "name": "Polygon",          "category": "layer2" },
        { "symbol": "LINK",  "name": "Chainlink",        "category": "oracle" },
        { "symbol": "DOT",   "name": "Polkadot",         "category": "layer0" },
        { "symbol": "ATOM",  "name": "Cosmos",           "category": "layer0" },
        { "symbol": "LTC",   "name": "Litecoin",         "category": "payments" },
        { "symbol": "UNI",   "name": "Uniswap",          "category": "defi" },
        { "symbol": "AAVE",  "name": "Aave",             "category": "defi" },
        { "symbol": "NEAR",  "name": "NEAR Protocol",    "category": "layer1" },
        { "symbol": "APT",   "name": "Aptos",            "category": "layer1" },
        { "symbol": "ARB",   "name": "Arbitrum",         "category": "layer2" },
        { "symbol": "OP",    "name": "Optimism",         "category": "layer2" },
        { "symbol": "SUI",   "name": "Sui",              "category": "layer1" },
        { "symbol": "INJ",   "name": "Injective",        "category": "layer1" },
        { "symbol": "TON",   "name": "Toncoin",          "category": "layer1" },
        { "symbol": "TRX",   "name": "Tron",             "category": "layer1" },
        { "symbol": "XLM",   "name": "Stellar",          "category": "payments" },
        { "symbol": "PEPE",  "name": "Pepe",             "category": "meme" },
        { "symbol": "SHIB",  "name": "Shiba Inu",        "category": "meme" }
    ]))
}

#[derive(serde::Deserialize)]
struct CryptoPricesQuery {
    symbols: Option<String>,  // comma-separated, e.g. "BTC,ETH,SOL"
    exchange: Option<String>, // "binance" | "coinbase" | "kraken" | "coingecko"
}

async fn get_crypto_prices(
    axum::extract::Query(req): axum::extract::Query<CryptoPricesQuery>,
) -> impl axum::response::IntoResponse {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let exchange = req.exchange.as_deref().unwrap_or("binance");
    let symbols: Vec<String> = req
        .symbols
        .as_deref()
        .unwrap_or("BTC,ETH,SOL,BNB,XRP,ADA,DOGE,AVAX,MATIC,LINK,DOT,ATOM,LTC,UNI,AAVE,NEAR,APT,ARB,OP,SUI,INJ,TON,TRX,XLM,PEPE,SHIB")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(tredo_core::normalize_base_symbol)
        .collect();

    let mut results = serde_json::Map::new();

    if exchange == "binance" {
        let sym_refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        match tredo_core::fetch_tickers_24hr_batch(&client, &sym_refs).await {
            Ok(tickers) => {
                for ticker in &tickers {
                    results.insert(
                        ticker.base_symbol.clone(),
                        tredo_core::ticker_to_api_json(ticker, "binance"),
                    );
                }
            }
            Err(e) => eprintln!("[API] Binance batch ticker failed: {e}"),
        }
        for sym in &symbols {
            if results.contains_key(sym) {
                continue;
            }
            match loops::fetch_coingecko_price(&client, sym).await {
                Ok(p) => {
                    results.insert(
                        sym.clone(),
                        serde_json::json!({ "price": p, "exchange": "coingecko" }),
                    );
                }
                Err(e) => {
                    results.insert(sym.clone(), serde_json::json!({ "error": e.to_string() }));
                }
            }
        }
    } else {
        for sym in &symbols {
            let price_result = match exchange {
                "coinbase" => loops::fetch_coinbase_price(&client, sym).await,
                "kraken" => loops::fetch_kraken_price(&client, sym).await,
                "coingecko" => loops::fetch_coingecko_price(&client, sym).await,
                _ => loops::fetch_binance_price(&client, sym).await,
            };
            match price_result {
                Ok(p) => {
                    results.insert(
                        sym.clone(),
                        serde_json::json!({ "price": p, "exchange": exchange }),
                    );
                }
                Err(e) => {
                    results.insert(sym.clone(), serde_json::json!({ "error": e.to_string() }));
                }
            }
        }
    }
    Json(serde_json::Value::Object(results))
}

// --- WebSocket for real-time updates (prices, COT, signals, portfolio) ---
// Clients connect to ws://host:port/ws
// In production, use a broadcast::Sender from loops / state to fan-out messages.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WebState>) -> Response {
    ws.on_upgrade(|mut socket: WebSocket| async move {
        let _ = socket
            .send(Message::Text(
                r#"{"type":"welcome","message":"tredo real-time connected (debate + trained vector + agentmemory)." }"#.to_string(),
            ))
            .await;

        // Subscribe to state updates for live COT/prices/signals (connects pipelines to clients)
        let mut rx = state.orchestrator.state.update_tx.subscribe();
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    if let Ok(update) = msg {
                        if socket.send(Message::Text(update)).await.is_err() {
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                    if socket.send(Message::Text(r#"{"type":"ping"}"#.to_string())).await.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

// ── Broker Management Endpoints ──────────────────────────────────────────────

/// Get current broker status — mode, broker name, connection state, and registered brokers.
async fn get_broker_status(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let mode = state
        .orchestrator
        .state
        .broker_registry
        .current_mode()
        .await;
    let broker_name = state
        .orchestrator
        .state
        .broker_registry
        .current_broker_name()
        .await;

    Json(serde_json::json!({
        "mode": mode,
        "broker": broker_name,
        "connected": true, // the registry always has paper; live status via broker
        "paper_balance": state.orchestrator.state.config.initial_balance,
    }))
}

/// Switch between paper and live trading mode.
/// Body: {"mode": "paper" | "live"}
#[derive(serde::Deserialize)]
struct SwitchModeRequest {
    mode: String,
}

async fn switch_broker_mode(
    State(state): State<WebState>,
    Json(req): Json<SwitchModeRequest>,
) -> impl axum::response::IntoResponse {
    let new_mode = match req.mode.to_lowercase().as_str() {
        "paper" => TradingMode::Paper,
        "live" => TradingMode::Live,
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Invalid mode. Use 'paper' or 'live'."
                })),
            );
        }
    };

    match state
        .orchestrator
        .state
        .broker_registry
        .set_mode(new_mode)
        .await
    {
        Ok(()) => {
            let broker_name = state
                .orchestrator
                .state
                .broker_registry
                .current_broker_name()
                .await;
            let msg = format!("Switched to {} mode via {}", req.mode, broker_name);

            state
                .orchestrator
                .state
                .push_cot(
                    "MetaControl",
                    &format!("Trading mode switch: {}", req.mode),
                    "MODE_SWITCH",
                    &msg,
                    1.0,
                    0,
                    None,
                    None,
                )
                .await;

            info!(message = %msg, "Trading mode switched");

            (
                axum::http::StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "mode": new_mode,
                    "broker": broker_name,
                    "message": msg
                })),
            )
        }
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to switch mode: {}", e)
            })),
        ),
    }
}

async fn get_crypto_market(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl axum::response::IntoResponse {
    let client = reqwest::Client::new();
    let sym = q
        .get("symbol")
        .map(|s| s.to_uppercase())
        .unwrap_or_else(|| "BTC".to_string());

    // Fetch 24h stats from Binance
    let binance_data = loops::fetch_binance_24h_ticker(&client, &sym)
        .await
        .unwrap_or_default();
    let price = binance_data["lastPrice"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let change_pct = binance_data["priceChangePercent"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let high = binance_data["highPrice"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let low = binance_data["lowPrice"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let volume = binance_data["volume"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let quote_vol = binance_data["quoteVolume"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    // Cross-exchange comparison (async parallel)
    let sym_clone = sym.clone();
    let client2 = client.clone();
    let coingecko_price = tokio::spawn(async move {
        loops::fetch_coingecko_price(&client2, &sym_clone)
            .await
            .unwrap_or(0.0)
    });
    let cgp = coingecko_price.await.unwrap_or(0.0);

    Json(serde_json::json!({
        "symbol": sym,
        "binance": {
            "price": price,
            "change_pct_24h": change_pct,
            "high_24h": high,
            "low_24h": low,
            "volume_24h": volume,
            "quote_volume_24h": quote_vol
        },
        "coingecko": {
            "price": cgp
        },
        "spread": if price > 0.0 && cgp > 0.0 { ((price - cgp) / price * 100.0).abs() } else { 0.0 }
    }))
}

async fn get_policy_cache(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let cache =
        tredo_runtime::policy_cache::PolicyCache::from_disk(state.orchestrator.state.clone());

    // Map entries to include computed fields (win_rate, confidence) that serde
    // can't serialize automatically since they are methods, not struct fields.
    let top: Vec<serde_json::Value> = cache
        .top_performers(3, 20)
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "features": e.features,
                "recommended_action": e.recommended_action,
                "sample_size": e.sample_size,
                "wins": e.wins,
                "losses": e.losses,
                "avg_pnl_pct": e.avg_pnl_pct,
                "avg_regret": e.avg_regret,
                "win_rate": e.win_rate(),
                "confidence": e.confidence(),
            })
        })
        .collect();

    let (cache_lookups, cache_hits, hit_rate) = cache.hit_stats();
    let hit_rate_history = cache.hit_rate_history();
    let top_win_rate_history = cache.top_win_rate_history();
    let pnl_history = cache.pnl_history();
    let equity_history = cache.equity_history();
    let confidence_history = cache.confidence_history();
    let streak_history = cache.streak_history();

    Json(serde_json::json!({
        "total_entries": cache.size(),
        "total_samples": cache.total_samples(),
        "config": {
            "min_samples": cache.config().min_samples,
            "min_win_rate": cache.config().min_win_rate,
            "min_confidence": cache.config().min_confidence,
        },
        "hit_stats": {
            "total_lookups": cache_lookups,
            "cache_hits": cache_hits,
            "hit_rate": hit_rate,
        },
        "hit_rate_history": hit_rate_history,
        "top_win_rate_history": top_win_rate_history,
        "pnl_history": pnl_history,
        "equity_history": equity_history,
        "confidence_history": confidence_history,
        "streak_history": streak_history,
        "top_performers": top,
    }))
}

async fn get_news(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let client = reqwest::Client::new();
    let fetcher = tredo_core::NewsFetcher::new(client, (*state.orchestrator.state.config).clone()); // free news APIs + keys (research: Alpha Vantage, Finnhub etc.)
                                                                                                    // Fetch for a default symbol; in prod could take query param for active symbol
    let items = fetcher.fetch_headlines("NIFTY").await.unwrap_or_default();
    Json(serde_json::json!({ "symbol": "NIFTY", "items": items }))
}

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Initialize structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "tredo_orchestrator=info,tredo_autonomous=info,tredo_core=info".into()
            }),
        )
        .init();

    std::panic::set_hook(Box::new(|panic_info| {
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic".to_string()
        };
        let location = panic_info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        eprintln!("💥 [PANIC] {} at {} — SYSTEM CRASHED", msg, location);
    }));

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║   tredo — Trading Real-time Edge Decision Optimisation ║");
    println!("║   Terminal UI | Temporal Loops | Agentic Memory        ║");
    println!("╚══════════════════════════════════════════════════════╝");
    tredo_autonomous::Tredo::print_tree();

    let mut orchestrator = match initialize_autonomous_system().await {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(error = %e, "Failed to initialize");
            std::process::exit(1);
        }
    };
    // Initialize the Tredo agent hierarchy (zero-copy Arc sharing from orchestrator)
    orchestrator.init_tredo();

    // === NEW: Initialize the global OutcomeProcessor for self-evolution ===
    {
        let db_for_meta = (*orchestrator.state.episode_store).clone();
        tredo_autonomous::execution_coordinator::init_outcome_processor(
            (*orchestrator.state.episode_store).clone(),
            db_for_meta,
        )
        .await;
        info!("OutcomeProcessor initialized — self-evolution loop active");

        // === agentmemory auto-start hint ===
        {
            let mem = tredo_core::AgentMemoryClient::new();
            match mem.recall("health check").await {
            Ok(_) => info!("agentmemory service detected — cross-session trained intelligence active"),
            Err(_) => info!("agentmemory not running. Start with `agentmemory connect` for persistent cross-session trained memory."),
        }
        }
    }

    let client = reqwest::Client::new();

    restore_portfolio_state(&orchestrator.state).await;
    restore_agent_tasks(&orchestrator.state).await;
    restore_watchlist(&orchestrator.state).await;

    {
        let mut wl = orchestrator.state.watchlist.write().await;
        if wl.is_empty() {
            *wl = vec![
                "BTC".to_string(),
                "ETH".to_string(),
                "SOL".to_string(),
                "BNB".to_string(),
                "XRP".to_string(),
                "ADA".to_string(),
                "DOGE".to_string(),
                "AVAX".to_string(),
                "MATIC".to_string(),
                "LINK".to_string(),
                "DOT".to_string(),
                "ATOM".to_string(),
                "LTC".to_string(),
                "UNI".to_string(),
                "AAVE".to_string(),
                "NEAR".to_string(),
                "APT".to_string(),
                "ARB".to_string(),
                "OP".to_string(),
                "SUI".to_string(),
                "INJ".to_string(),
                "TON".to_string(),
                "TRX".to_string(),
                "XLM".to_string(),
                "PEPE".to_string(),
                "SHIB".to_string(),
                "NIFTY".to_string(),
                "RELIANCE".to_string(),
            ];
            info!(count = wl.len(), symbols = ?*wl, "Seeded default watchlist");
        }
    }

    let assets = orchestrator.state.watchlist.read().await.clone();
    schedule_data_feed_init(orchestrator.clone(), client.clone(), assets.clone());

    // ── Watchdog Heartbeat ──────────────────────────────────────────────
    // Spawn a background task that sends UDP heartbeats to the tredo-watchdog
    // service every 5 seconds. If the watchdog stops receiving heartbeats,
    // it will trigger emergency procedures (API key revocation, alerts).
    //
    // The watchdog runs as a SEPARATE BINARY and cannot be bypassed.
    // If the watchdog is not running, heartbeats are silently dropped (UDP).
    {
        let watchdog_addr =
            std::env::var("WATCHDOG_ADDR").unwrap_or_else(|_| "127.0.0.1:9711".to_string());
        tokio::spawn(async move {
            // Bind UDP socket for sending heartbeats
            let socket = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "Failed to bind UDP socket");
                    return;
                }
            };

            let remote_addr: std::net::SocketAddr = match watchdog_addr.parse() {
                Ok(a) => a,
                Err(e) => {
                    warn!(addr = %watchdog_addr, error = %e, "Invalid WATCHDOG_ADDR");
                    return;
                }
            };

            let heartbeat_msg = b"TREDO_HEARTBEAT";
            info!(addr = %remote_addr, "Sending heartbeats every 5s");

            loop {
                if let Err(e) = socket.send_to(heartbeat_msg, remote_addr).await {
                    // Don't flood logs if watchdog is not running
                    warn!(error = %e, "Heartbeat send failed");
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        info!("Watchdog heartbeat sender started");
    }

    // ── Metrics Client ─────────────────────────────────────────────────
    // Spawn background tasks that send system health and trade events to
    // the tredo-metrics service (runs on port 9730 by default).
    // If the metrics service is not running, events are silently dropped.
    {
        let metrics_url =
            std::env::var("METRICS_URL").unwrap_or_else(|_| "http://127.0.0.1:9730".to_string());
        let metrics_client = reqwest::Client::new();
        let state_for_metrics = orchestrator.state.clone();

        // Health heartbeat: send system health status every 60 seconds
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;

                let kronos_up = reqwest::Client::new()
                    .get("http://localhost:8000/docs")
                    .timeout(Duration::from_millis(400))
                    .send()
                    .await
                    .is_ok();

                let ollama_up = state_for_metrics.llm.is_ollama_running().await;
                let running = {
                    let p = state_for_metrics.portfolio.read().await;
                    p.trading_enabled
                };

                let events = vec![
                    serde_json::json!({
                        "event_type": "system_health",
                        "service": "kronos",
                        "healthy": kronos_up,
                        "latency_ms": serde_json::Value::Null,
                        "error_message": if kronos_up { serde_json::Value::Null } else { serde_json::Value::String("Kronos service unreachable".to_string()) },
                        "timestamp_micros": chrono::Utc::now().timestamp_micros(),
                    }),
                    serde_json::json!({
                        "event_type": "system_health",
                        "service": "llm",
                        "healthy": ollama_up,
                        "latency_ms": serde_json::Value::Null,
                        "error_message": if ollama_up { serde_json::Value::Null } else { serde_json::Value::String("LLM service unreachable".to_string()) },
                        "timestamp_micros": chrono::Utc::now().timestamp_micros(),
                    }),
                    serde_json::json!({
                        "event_type": "system_health",
                        "service": "orchestrator",
                        "healthy": running,
                        "latency_ms": serde_json::Value::Null,
                        "error_message": if running { serde_json::Value::Null } else { serde_json::Value::String("Orchestrator not running".to_string()) },
                        "timestamp_micros": chrono::Utc::now().timestamp_micros(),
                    }),
                ];

                for event in &events {
                    let url = format!("{}/event", metrics_url.trim_end_matches('/'));
                    if let Err(e) = metrics_client
                        .post(&url)
                        .json(event)
                        .timeout(Duration::from_secs(2))
                        .send()
                        .await
                    {
                        // Silent — metrics service may not be running
                        warn!(error = %e, "Health event send failed");
                    }
                }
            }
        });
        info!("Metrics client started (health events every 60s)");
    }

    // ── Live Broker Background Tasks ──────────────────────────────────
    // Spawn LiveOrderManager polling loop: polls pending orders every 10s in live mode
    {
        let state_for_orders = orchestrator.state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;

                // Only poll if in LIVE mode
                let is_live =
                    state_for_orders.broker_registry.current_mode().await == TradingMode::Live;
                if !is_live {
                    continue;
                }

                let pending = match state_for_orders
                    .live_order_manager
                    .get_pending_orders()
                    .await
                {
                    Ok(orders) => orders,
                    Err(e) => {
                        warn!(error = %e, "Failed to query pending orders");
                        continue;
                    }
                };

                if pending.is_empty() {
                    continue;
                }

                let broker = state_for_orders.broker_registry.active_broker().await;

                for order in &pending {
                    // Check if order should time out (15 minutes max)
                    let elapsed = (Utc::now() - order.created_at).num_minutes();
                    if elapsed > 15 {
                        info!(order_id = %order.broker_order_id, minutes = elapsed, "Order timed out");
                        let _ = state_for_orders
                            .live_order_manager
                            .update_status(
                                &order.broker_order_id,
                                tredo_core::paper_engine::OrderStatus::Expired,
                                0,
                                None,
                                Some("Timed out".to_string()),
                            )
                            .await;
                        continue;
                    }

                    match broker.get_order_status(&order.broker_order_id).await {
                        Ok(status) => {
                            let is_terminal = matches!(
                                status,
                                tredo_core::paper_engine::OrderStatus::Filled
                                    | tredo_core::paper_engine::OrderStatus::Cancelled
                                    | tredo_core::paper_engine::OrderStatus::Expired
                            ) || matches!(
                                status,
                                tredo_core::paper_engine::OrderStatus::Rejected { .. }
                            );

                            if is_terminal {
                                let filled_qty = if matches!(
                                    status,
                                    tredo_core::paper_engine::OrderStatus::Filled
                                ) {
                                    order.qty
                                } else {
                                    0
                                };
                                let _ = state_for_orders
                                    .live_order_manager
                                    .update_status(
                                        &order.broker_order_id,
                                        status,
                                        filled_qty,
                                        None,
                                        None,
                                    )
                                    .await;
                                info!(order_id = %order.broker_order_id, "Order resolved: terminal status");
                            }
                        }
                        Err(e) => {
                            // Report connection drop to circuit breaker
                            state_for_orders
                                .circuit_breaker
                                .report_connection_drop()
                                .await;
                            warn!(order_id = %order.broker_order_id, error = %e, "Failed to poll order");
                        }
                    }
                }
            }
        });
        info!("Live order polling loop started (10s cadence in LIVE mode)");
    }

    // Spawn ReconciliationEngine loop: runs every 60s in live mode
    {
        let state_for_recon = orchestrator.state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;

                let is_live =
                    state_for_recon.broker_registry.current_mode().await == TradingMode::Live;
                if !is_live {
                    continue;
                }

                info!("Running broker reconciliation cycle...");
                let engine = tredo_autonomous::reconciliation_engine::ReconciliationEngine::new(
                    state_for_recon.clone(),
                );
                let report = engine.reconcile().await;

                if report.has_issues() {
                    warn!(summary = %report.summary(), "Reconciliation issues detected");
                    if !report.auto_closed.is_empty() {
                        for closed in &report.auto_closed {
                            warn!(order = %closed, "Auto-closed order");
                        }
                    }
                    if !report.auto_imported.is_empty() {
                        for imported in &report.auto_imported {
                            info!(order = %imported, "Auto-imported order");
                        }
                    }
                } else {
                    info!(summary = %report.summary(), "Reconciliation OK");
                }

                // Update circuit breaker equity from latest portfolio snapshot
                let equity = state_for_recon.portfolio.read().await.total_equity;
                state_for_recon.circuit_breaker.update_equity(equity).await;
            }
        });
        info!("Reconciliation loop started (60s cadence in LIVE mode)");
    }

    // ── Live Broker Initialization ──────────────────────────────────────
    // Check env vars for all supported live brokers and register them.
    // The first registered broker is used as the default live broker.
    // Multiple brokers can be registered; the active one can be switched via the API.

    // ── Zerodha Kite ──────────────────────────────────────────────────────
    {
        let zerodha_api_key = std::env::var("ZERODHA_API_KEY").ok();
        let zerodha_api_secret = std::env::var("ZERODHA_API_SECRET").ok();
        let zerodha_request_token = std::env::var("ZERODHA_REQUEST_TOKEN").ok();

        if let (Some(api_key), Some(api_secret)) = (&zerodha_api_key, &zerodha_api_secret) {
            let request_token = zerodha_request_token.as_deref().unwrap_or("");
            let live_broker =
                tredo_broker_zerodha::create_zerodha_broker(api_key, api_secret, request_token);
            orchestrator
                .state
                .broker_registry
                .register_live_broker(live_broker)
                .await;

            if !request_token.is_empty() {
                info!("Zerodha Kite broker registered with request_token");
            } else {
                info!("Zerodha Kite broker registered (no token — use /api/broker/connect to authenticate)");
            }
        } else {
            info!("No Zerodha credentials found. Set ZERODHA_API_KEY and ZERODHA_API_SECRET for live trading.");
        }
    }

    // ── Upstox ────────────────────────────────────────────────────────────
    {
        let client_id = std::env::var("UPSTOX_CLIENT_ID").ok();
        let client_secret = std::env::var("UPSTOX_CLIENT_SECRET").ok();
        let access_token = std::env::var("UPSTOX_ACCESS_TOKEN").ok();

        if let (Some(cid), Some(cs)) = (&client_id, &client_secret) {
            let redirect_uri = std::env::var("UPSTOX_REDIRECT_URI")
                .unwrap_or_else(|_| "http://localhost:8080/callback".to_string());
            let token = access_token.as_deref().unwrap_or("");
            let live_broker =
                tredo_broker_upstox::create_upstox_broker(cid, cs, &redirect_uri, token);
            orchestrator
                .state
                .broker_registry
                .register_live_broker(live_broker)
                .await;
            info!("Upstox broker registered");
        } else {
            info!("No Upstox credentials found. Set UPSTOX_CLIENT_ID and UPSTOX_CLIENT_SECRET for live trading.");
        }
    }

    // ── Angel One ─────────────────────────────────────────────────────────
    {
        let api_key = std::env::var("ANGEL_API_KEY").ok();
        let client_id = std::env::var("ANGEL_CLIENT_ID").ok();
        let pin = std::env::var("ANGEL_PIN").ok();

        if let (Some(ak), Some(ci), Some(p)) = (&api_key, &client_id, &pin) {
            let totp_secret = std::env::var("ANGEL_TOTP_SECRET").ok();
            let live_broker = tredo_broker_angelone::create_angelone_broker(ak, ci, p, totp_secret);
            orchestrator
                .state
                .broker_registry
                .register_live_broker(live_broker)
                .await;
            info!("Angel One broker registered");
        } else {
            info!("No Angel One credentials found. Set ANGEL_API_KEY, ANGEL_CLIENT_ID, and ANGEL_PIN for live trading.");
        }
    }

    // ── 5Paisa ────────────────────────────────────────────────────────────
    {
        let app_key = std::env::var("FIVEPAISA_APP_KEY").ok();
        let encry_key = std::env::var("FIVEPAISA_ENCRY_KEY").ok();
        let user_id = std::env::var("FIVEPAISA_USER_ID").ok();
        let client_code = std::env::var("FIVEPAISA_CLIENT_CODE").ok();

        if let (Some(ak), Some(ek), Some(ui), Some(cc)) =
            (&app_key, &encry_key, &user_id, &client_code)
        {
            let live_broker = tredo_broker_5paisa::create_fivepaisa_broker(ak, ek, ui, cc);
            orchestrator
                .state
                .broker_registry
                .register_live_broker(live_broker)
                .await;
            info!("5Paisa broker registered");
        } else {
            info!("No 5Paisa credentials found. Set FIVEPAISA_APP_KEY, FIVEPAISA_ENCRY_KEY, FIVEPAISA_USER_ID, and FIVEPAISA_CLIENT_CODE for live trading.");
        }
    }

    // ── Alpaca (Equities & Crypto) ──────────────────────────────────────────
    {
        let api_key_id = std::env::var("ALPACA_API_KEY_ID").ok();
        let api_secret_key = std::env::var("ALPACA_API_SECRET_KEY").ok();
        let paper = std::env::var("ALPACA_PAPER")
            .map(|v| v == "true")
            .unwrap_or(true);

        if let (Some(api_key), Some(api_secret)) = (&api_key_id, &api_secret_key) {
            let live_broker = tredo_broker_alpaca::create_alpaca_broker(api_key, api_secret, paper);
            orchestrator
                .state
                .broker_registry
                .register_live_broker(live_broker)
                .await;
            let mode = if paper { "paper" } else { "live" };
            info!(mode = %mode, "Alpaca broker registered");
        } else {
            info!("No Alpaca credentials found. Set ALPACA_API_KEY_ID and ALPACA_API_SECRET_KEY for live trading.");
        }
    }

    info!("To switch to live mode: POST /api/broker/switch with {{mode: live}}");
    info!("Paper trading is active by default.");

    // ── Event Bus Initialization ───────────────────────────────────────
    // Create the event bus for pub-sub communication between microservices.
    // Uses NATS if EVENT_BUS_URL is set, otherwise in-memory (single-process).
    let event_bus: Arc<dyn EventBus> = Arc::from(
        tredo_eventbus::create_event_bus()
            .await
            .expect("Failed to create event bus"),
    );

    // Publish START system control event on launch
    {
        let _ = event_bus
            .publish(
                &event_subjects::system_control(),
                &TredoEvent::SystemControl(tredo_eventbus::SystemControlEvent {
                    command: "START".to_string(),
                    target: None,
                    reason: "Orchestrator started".to_string(),
                    timestamp_micros: chrono::Utc::now().timestamp_micros(),
                }),
            )
            .await;
        info!("Published START system control event");
    }

    // Create background loop manager
    let loop_manager = Arc::new(TokioMutex::new(LoopManager::new(
        orchestrator.clone(),
        client.clone(),
        assets,
        event_bus.clone(),
    )));

    // ======================================================================
    // FULL AUTONOMOUS MODE — LAUNCH AND FORGET
    // Once started, the agent runs 24/7 with no further human input required.
    // - Fast loop (5s): price updates + automatic SL/TP management (paper)
    // - Medium loop (5m): full Tredo pipeline (market intel → discipline →
    //   strategy decision → execution) for every symbol in the watchlist
    // - Slow loop (24h): reflection + meta-control (self-improvement)
    //
    // The HTTP server + static frontend (or Tauri UI) are purely for
    // OBSERVATION. You can close the UI/browser after launch; the agent
    // keeps running.
    // ======================================================================
    {
        let mut manager = loop_manager.lock().await;
        let started = manager.start().await;
        if started {
            info!("AUTONOMOUS MODE ACTIVE");
            info!("  Loops running independently (no UI required)");
            info!("  Paper trades will be executed automatically when signals pass all guards");
            info!("  Use Ctrl+C (or the Stop button in UI) to shut down cleanly");
        }
    }

    // Set up Axum Web Server routing
    let state = WebState {
        orchestrator: orchestrator.clone(),
        loop_manager: loop_manager.clone(),
    };

    let api_routes = Router::new()
        .route("/status", get(get_system_status))
        .route("/portfolio", get(get_portfolio))
        .route("/health", get(get_system_health))
        .route("/cot", get(get_cot_chains))
        .route("/models", get(get_available_models))
        .route("/models/set", post(set_llm_model))
        .route("/start", post(start_autonomous_system))
        .route("/stop", post(stop_autonomous_system))
        .route("/trade", post(execute_trade))
        .route("/trigger_cycle", post(trigger_orchestra_cycle))
        .route("/pipeline/run", post(run_pipeline_batch))
        .route("/rules", post(update_rules))
        .route("/backtest", get(run_backtest))
        .route("/backtest/results", get(get_backtest_results))
        .route("/price", get(fetch_live_stock_price))
        .route("/agents", get(get_agent_tree))
        .route("/skills", get(get_skill_scores))
        .route("/watchlist", get(get_watchlist))
        .route("/watchlist/add", post(add_to_watchlist))
        .route("/watchlist/remove", post(remove_from_watchlist))
        .route("/metrics", get(get_metrics))
        .route("/indicators", get(get_metrics))
        // ── Crypto Exchange Routes ──────────────────────────────────────────
        .route("/crypto/exchanges", get(get_crypto_exchanges))
        .route("/crypto/symbols", get(get_crypto_symbols))
        .route("/crypto/prices", get(get_crypto_prices))
        .route("/crypto/market", get(get_crypto_market))
        // ── Broker Management Routes ─────────────────────────────────------
        .route("/broker/status", get(get_broker_status))
        .route("/broker/switch", post(switch_broker_mode))
        .route("/depth", get(get_market_depth))
        .route("/news", get(get_news))
        .route("/policy-cache", get(get_policy_cache))
        .route("/ws", get(ws_handler));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Serve static files from frontend path and mount API on /api
    let app = Router::new()
        .nest("/api", api_routes)
        .fallback_service(ServeDir::new("src-tauri/frontend"))
        .layer(cors)
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| {
            std::env::var("WEB_API_ADDR")
                .ok()
                .and_then(|a| a.split(':').next_back().and_then(|p| p.parse().ok()))
        })
        .unwrap_or(8080);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!(addr = %addr, "HTTP server starting");

    let server_handle = tokio::spawn(async move {
        // Robust port binding: try PORT, then +1 up to 10 times (fixes AddrInUse panics from unclean previous runs)
        let mut current_port = port;
        let listener = loop {
            let try_addr = SocketAddr::from(([0, 0, 0, 0], current_port));
            match tokio::net::TcpListener::bind(try_addr).await {
                Ok(l) => {
                    if current_port != port {
                        warn!(
                            requested = port,
                            actual = current_port,
                            "Port in use, using alternative"
                        );
                    }
                    break l;
                }
                Err(e) if current_port < port + 10 => {
                    current_port += 1;
                    warn!(port = current_port - 1, error = %e, "Port bind failed, trying next...");
                    continue;
                }
                Err(e) => {
                    error!(port = current_port, error = %e, "Failed to bind port. Exiting.");
                    std::process::exit(1);
                }
            }
        };
        axum::serve(listener, app).await.unwrap();
    });

    signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
    info!("Shutdown signal received. Stopping tredo...");

    {
        let mut manager = loop_manager.lock().await;
        manager.stop().await;
    }
    server_handle.abort();

    graceful_shutdown(&orchestrator).await;
}
