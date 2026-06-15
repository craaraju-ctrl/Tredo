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
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::{watch, Mutex as TokioMutex};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tredo_autonomous::state::initialize_autonomous_system;

// ── Loop Manager to dynamically start and stop the background temporal loops ──
struct LoopManager {
    orchestrator: tredo_autonomous::AutonomousOrchestrator,
    client: reqwest::Client,
    assets: Vec<String>,
    shutdown_tx: Option<watch::Sender<bool>>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl LoopManager {
    fn new(
        orchestrator: tredo_autonomous::AutonomousOrchestrator,
        client: reqwest::Client,
        assets: Vec<String>,
    ) -> Self {
        Self {
            orchestrator,
            client,
            assets,
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

        let orch_medium = self.orchestrator.clone();
        let client_medium = self.client.clone();
        let assets_medium = self.assets.clone();
        let rx_medium = rx.clone();

        let orch_slow = self.orchestrator.clone();
        let state_slow = self.orchestrator.state.clone();
        let rx_slow = rx.clone();

        let fast_handle = tokio::spawn(async move {
            loops::fast_loop(orch_fast, client_fast, assets_fast, rx_fast).await;
        });

        let medium_handle = tokio::spawn(async move {
            loops::medium_loop(orch_medium, client_medium, assets_medium, rx_medium).await;
        });

        let slow_handle = tokio::spawn(async move {
            loops::slow_loop(orch_slow, state_slow, rx_slow).await;
        });

        self.handles = vec![fast_handle, medium_handle, slow_handle];

        {
            let mut p = self.orchestrator.state.portfolio.write().await;
            p.trading_enabled = true;
        }

        println!("[Orchestrator] 🚀 Background loops started.");
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
            println!("[Orchestrator] 🛑 Background loops stopped cleanly.");
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

async fn initialize_system(
    orchestrator: &tredo_autonomous::AutonomousOrchestrator,
    client: &reqwest::Client,
) -> Vec<String> {
    let assets = orchestrator.state.watchlist.read().await.clone();
    println!(
        "[Orchestrator] 🌐 Initializing all data feeds for watchlist: {:?}",
        assets
    );

    for symbol in &assets {
        let is_crypto = loops::is_crypto_symbol(symbol);

        let bars = if is_crypto {
            loops::fetch_binance_klines(client, symbol, "1m", 100)
                .await
                .unwrap_or_default()
        } else {
            loops::fetch_yahoo_ohlcv(client, symbol)
                .await
                .unwrap_or_default()
        };
        if !bars.is_empty() {
            let mut history = orchestrator.state.ohlcv_history.write().await;
            history.insert(symbol.clone(), bars);
        }

        loops::update_multi_tf_data(client, orchestrator, symbol, is_crypto).await;
    }

    {
        let mut summary = orchestrator.state.agent_market_summary.write().await;
        let monitored = if assets.is_empty() {
            "No whitelisted assets".to_string()
        } else {
            assets.join(", ")
        };
        *summary = format!(
            "System initialized. Monitoring: {} with Ollama + Kronos.",
            monitored
        );
    }

    println!("[Orchestrator] ✅ System initialized — Web API ready");
    assets
}

async fn restore_portfolio_state(state: &tredo_autonomous::state::SharedState) -> bool {
    match state.memory.load_state("portfolio/state") {
        Ok(Some(json)) => {
            match serde_json::from_str::<tredo_autonomous::types::PortfolioState>(&json) {
                Ok(restored) => {
                    let mut portfolio = state.portfolio.write().await;
                    *portfolio = restored;
                    println!("[Restore] ✅ Portfolio restored — Equity: ₹{:.2} | Cash: ₹{:.2} | Positions: {}",
                        portfolio.total_equity, portfolio.cash_balance, portfolio.open_positions.len());
                    true
                }
                Err(e) => {
                    eprintln!("[Restore] ⚠ Failed to parse portfolio state: {e}. Starting fresh.");
                    false
                }
            }
        }
        Ok(None) => {
            println!("[Restore] ℹ No saved portfolio state found. Starting fresh.");
            false
        }
        Err(e) => {
            eprintln!("[Restore] ⚠ Failed to load portfolio state: {e}. Starting fresh.");
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
            println!("[Restore] ✅ Agent tasks restored from redb.");
        }
    }
}

// ── Graceful Shutdown ───────────────────────────────────────────────────────

async fn graceful_shutdown(orchestrator: &tredo_autonomous::AutonomousOrchestrator) {
    println!("\n[Shutdown] 🛑 Winding down web server & portfolio state...");
    loops::save_portfolio_state(&orchestrator.state).await;
    save_watchlist(&orchestrator.state).await;
    let p = orchestrator.state.portfolio.read().await;
    println!("[Shutdown] Final Portfolio — Equity: ₹{:.2} | P&L: ₹{:.2} | Trades: {} | Positions open: {}",
        p.total_equity, p.daily_pnl, p.total_trades_today, p.open_positions.len());
    drop(p);
    println!("[Shutdown] 👋 tredo terminated. Goodbye.");
}

// ── Axum Endpoint Handlers ───────────────────────────────────────────────────

async fn get_system_status(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let portfolio = state.orchestrator.state.portfolio.read().await;
    let rules = state.orchestrator.state.rules.read().await;
    Json(serde_json::json!({
        "status": "tredo Running",
        "initial_balance": state.orchestrator.state.config.initial_balance,
        "use_confluence": rules.use_confluence,
        "cash_balance": portfolio.cash_balance,
        "total_equity": portfolio.total_equity,
    }))
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

    // Set the model - need to create new LlmExecutor with new model
    // Since Arc<LlmExecutor> is immutable, we need to inform the agent about the change
    let old_model = state.orchestrator.state.llm.get_model();
    std::env::set_var("OLLAMA_MODEL", model.clone());

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
    match state.memory.load_state("watchlist/state") {
        Ok(Some(json)) => {
            if let Ok(restored) = serde_json::from_str::<Vec<String>>(&json) {
                let mut watchlist = state.watchlist.write().await;
                *watchlist = restored;
                println!(
                    "[Restore] ✅ Watchlist restored from redb: {:?}",
                    *watchlist
                );
            }
        }
        _ => {
            println!("[Restore] ℹ No saved watchlist found. Starting with empty watchlist.");
            let mut watchlist = state.watchlist.write().await;
            watchlist.clear();
        }
    }
}

#[derive(serde::Deserialize)]
struct WatchlistRequest {
    symbol: String,
}

async fn get_watchlist(State(state): State<WebState>) -> impl axum::response::IntoResponse {
    let wl = state.orchestrator.state.watchlist.read().await;
    Json(wl.clone())
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
            loops::fetch_yahoo_ohlcv(&client, &symbol)
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

    let signal = TradeSignal {
        symbol: req.symbol.clone(),
        direction,
        entry_price: req.entry_price,
        stop_loss: req.stop_loss,
        take_profit: req.take_profit,
        position_size: 10.0,
        confidence_score: 0.85,
        confluence_score: 0.85,
        risk_reward_ratio: 2.0,
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
            let response = serde_json::json!({
                "success": true,
                "message": exec_log
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

async fn trigger_orchestra_cycle(
    State(state): State<WebState>,
    Json(req): Json<CycleRequest>,
) -> impl axum::response::IntoResponse {
    use tredo_core::TradeDirection;
    let sym = req.symbol.unwrap_or_else(|| "NIFTY".to_string());

    println!("[WebAPI] === FULL ORCHESTRA CYCLE TRIGGERED FROM HTTP API (agentic - no pre-supplied levels) ===");

    // Agentic trigger: only the symbol. The agent observes market data from state and decides
    // direction + its own entry/SL/TP using full analysis (indicators it computes, debate, memory, rules).
    match state.orchestrator.run_full_pipeline(&sym).await {
        Ok(summary) => {
            let action = summary
                .final_signal
                .map(|s| {
                    format!(
                        "{} {:.2}",
                        if s.direction == TradeDirection::Long {
                            "BUY"
                        } else {
                            "SELL"
                        },
                        s.entry_price
                    )
                })
                .unwrap_or_else(|| "HOLD".to_string());
            Json(serde_json::json!({
                "message": format!(
                    "ORCHESTRA CYCLE COMPLETE | Action: {} | Reason: {} | Duration: {}ms",
                    action, summary.reason, summary.total_duration_ms
                )
            }))
        }
        Err(e) => Json(serde_json::json!({
            "message": format!("ORCHESTRA CYCLE ERROR: {}", e)
        })),
    }
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

#[derive(serde::Deserialize)]
struct PriceQuery {
    symbol: String,
}

async fn get_agent_tree() -> impl axum::response::IntoResponse {
    Json(tredo_autonomous::Tredo::tree_json())
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
    let client = reqwest::Client::new();
    let exchange = req.exchange.as_deref().unwrap_or("binance");
    let symbols: Vec<&str> = req
        .symbols
        .as_deref()
        .unwrap_or("BTC,ETH,SOL,BNB,XRP,ADA,DOGE,AVAX")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let mut results = serde_json::Map::new();
    for sym in symbols {
        let price_result = match exchange {
            "coinbase" => loops::fetch_coinbase_price(&client, sym).await,
            "kraken" => loops::fetch_kraken_price(&client, sym).await,
            "coingecko" => loops::fetch_coingecko_price(&client, sym).await,
            _ => {
                // Try Binance first, then fallback to CoinGecko
                match loops::fetch_binance_price(&client, sym).await {
                    Ok(p) => Ok(p),
                    Err(_) => loops::fetch_coingecko_price(&client, sym).await,
                }
            }
        };
        match price_result {
            Ok(p) => {
                results.insert(
                    sym.to_string(),
                    serde_json::json!({ "price": p, "exchange": exchange }),
                );
            }
            Err(e) => {
                results.insert(
                    sym.to_string(),
                    serde_json::json!({ "error": e.to_string() }),
                );
            }
        }
    }
    Json(serde_json::Value::Object(results))
}

// --- WebSocket for real-time updates (prices, COT, signals, portfolio) ---
// Clients connect to ws://host:port/ws
// In production, use a broadcast::Sender from loops / state to fan-out messages.
#[allow(dead_code)]
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
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    if socket.send(Message::Text(r#"{"type":"ping"}"#.to_string())).await.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

// Example broadcast helper (call from loops when pushing COT or price updates)
pub async fn broadcast_update(_msg: &str) {
    // TODO: integrate with a tokio::sync::broadcast channel stored in WebState
    // or SharedState. Example: state.ws_tx.send(msg.to_string()).await.ok();
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
        eprintln!("\n💥 [PANIC] {} at {} — SYSTEM CRASHED", msg, location);
    }));

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║   tredo — Trading Real-time Edge Decision Optimisation ║");
    println!("║   Terminal UI | Temporal Loops | Agentic Memory        ║");
    println!("╚══════════════════════════════════════════════════════╝");
    tredo_autonomous::Tredo::print_tree();

    let mut orchestrator = match initialize_autonomous_system().await {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[FATAL] Failed to initialize: {e}");
            std::process::exit(1);
        }
    };
    // Initialize the Tredo agent hierarchy (zero-copy Arc sharing from orchestrator)
    orchestrator.init_tredo();
    let client = reqwest::Client::new();

    restore_portfolio_state(&orchestrator.state).await;
    restore_agent_tasks(&orchestrator.state).await;
    restore_watchlist(&orchestrator.state).await;

    let assets = initialize_system(&orchestrator, &client).await;

    // Auto-seed a sensible default watchlist on first run if empty.
    // This lets the agent start trading (paper) immediately with no extra setup.
    {
        let mut wl = orchestrator.state.watchlist.write().await;
        if wl.is_empty() {
            *wl = vec!["BTC".to_string(), "ETH".to_string(), "SOL".to_string()];
            println!("[Orchestrator] Seeded default watchlist: {:?}", *wl);
        }
    }

    // Create background loop manager
    let loop_manager = Arc::new(TokioMutex::new(LoopManager::new(
        orchestrator.clone(),
        client.clone(),
        assets,
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
            println!("[Orchestrator] ✅ AUTONOMOUS MODE ACTIVE");
            println!("[Orchestrator]    • Loops running independently (no UI required)");
            println!("[Orchestrator]    • Paper trades will be executed automatically when signals pass all guards");
            println!(
                "[Orchestrator]    • Use Ctrl+C (or the Stop button in UI) to shut down cleanly"
            );
        }
    }

    // Set up Axum Web Server routing
    let state = WebState {
        orchestrator: orchestrator.clone(),
        loop_manager: loop_manager.clone(),
    };

    let api_routes = Router::new()
        .route("/status", get(get_system_status))
        .route("/health", get(get_system_health))
        .route("/cot", get(get_cot_chains))
        .route("/models", get(get_available_models))
        .route("/models/set", post(set_llm_model))
        .route("/start", post(start_autonomous_system))
        .route("/stop", post(stop_autonomous_system))
        .route("/trade", post(execute_trade))
        // NOTE: /trigger_cycle is kept for debugging / manual testing only.
        // In normal autonomous operation the medium loop drives run_full_pipeline
        // on its own schedule. You do not need to call this after launch.
        .route("/trigger_cycle", post(trigger_orchestra_cycle))
        .route("/rules", post(update_rules))
        .route("/backtest", get(run_backtest))
        .route("/price", get(fetch_live_stock_price))
        .route("/agents", get(get_agent_tree))
        .route("/watchlist", get(get_watchlist))
        .route("/watchlist/add", post(add_to_watchlist))
        .route("/watchlist/remove", post(remove_from_watchlist))
        // ── Crypto Exchange Routes ──────────────────────────────────────────
        .route("/crypto/exchanges", get(get_crypto_exchanges))
        .route("/crypto/symbols", get(get_crypto_symbols))
        .route("/crypto/prices", get(get_crypto_prices))
        .route("/crypto/market", get(get_crypto_market))
        .route("/news", get(get_news));

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
    println!("[Orchestrator] 🌐 HTTP server starting on {}", addr);

    let server_handle = tokio::spawn(async move {
        // Robust port binding: try PORT, then +1 up to 10 times (fixes AddrInUse panics from unclean previous runs)
        let mut current_port = port;
        let listener = loop {
            let try_addr = SocketAddr::from(([0, 0, 0, 0], current_port));
            match tokio::net::TcpListener::bind(try_addr).await {
                Ok(l) => {
                    if current_port != port {
                        println!(
                            "[Orchestrator] ⚠ Port {} in use, using {} instead",
                            port, current_port
                        );
                    }
                    break l;
                }
                Err(e) if current_port < port + 10 => {
                    current_port += 1;
                    eprintln!(
                        "[Orchestrator] Port {} bind failed ({}), trying next...",
                        current_port - 1,
                        e
                    );
                    continue;
                }
                Err(e) => {
                    eprintln!("[Orchestrator] Failed to bind port: {}. Exiting.", e);
                    std::process::exit(1);
                }
            }
        };
        axum::serve(listener, app).await.unwrap();
    });

    signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
    println!("\n[Main] 🛑 Shutdown signal received. Stopping tredo...");

    {
        let mut manager = loop_manager.lock().await;
        manager.stop().await;
    }
    server_handle.abort();

    graceful_shutdown(&orchestrator).await;
}
