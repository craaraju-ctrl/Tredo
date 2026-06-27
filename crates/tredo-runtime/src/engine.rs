//! Runtime engine — the main orchestrator that wires everything together.

use crate::active_learner::ActiveLearner;
use crate::api_clients;
use crate::backtest_feed::BacktestFeed;
use crate::data_feed::DataFeed;
use crate::event_bus::{AgentEvent, EventBus};
use crate::goal_manager::GoalManager;
use crate::introspector::Introspector;
use crate::mode::{ModeConfig, TradingMode};
use crate::policy_cache::PolicyCache;
use crate::portfolio_reasoner::PortfolioReasoner;
use crate::risk_manager::RiskManager;
use crate::streaming_reasoner::StreamingReasoner;
use crate::world_model::WorldModelEngine;

use anyhow::Context;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, watch};
use tredo_autonomous::orchestrator_struct::AutonomousOrchestrator;
use tredo_core::paper_engine::{BrokerRegistry, OrderRequest, OrderType};
use tredo_core::TradeDirection;

/// Summary of a run returned by the engine.
pub struct RunSummary {
    pub mode: TradingMode,
    pub cycles_completed: usize,
    pub events_processed: u64,
    pub trades_executed: usize,
    pub cache_hits: usize,
    pub ollama_calls: usize,
    pub total_pnl: f64,
    pub max_drawdown: f64,
    pub duration_secs: u64,
    pub risk_summary: String,
    pub goal_summary: String,
}

/// The unified runtime engine.
pub struct RuntimeEngine {
    mode_config: ModeConfig,
    orchestrator: AutonomousOrchestrator,
    event_bus: EventBus,
    risk_manager: Arc<RiskManager>,
    introspector: Arc<Introspector>,
    goal_manager: GoalManager,
    world_model: Arc<WorldModelEngine>,
    #[allow(dead_code)]
    portfolio_reasoner: Arc<PortfolioReasoner>,
    #[allow(dead_code)]
    active_learner: Arc<ActiveLearner>,
    #[allow(dead_code)]
    streaming_reasoner: Arc<StreamingReasoner>,
    /// Learned policy cache — reduces LLM calls by caching (features → action → outcome).
    /// Seeded from historical trades on startup, updated after every closed trade.
    pub policy_cache: Arc<tokio::sync::Mutex<PolicyCache>>,
    /// Optional broker registry — if set and mode is Live, cache-hit trades
    /// are routed through the live broker adapter instead of the paper-only
    /// `ExecutionCoordinatorAgent::execute_paper_trade`.
    broker_registry: Option<Arc<BrokerRegistry>>,
    symbols: Vec<String>,
    cycles_completed: usize,
    total_pnl: f64,
    max_drawdown: f64,
    start_time: Instant,
}

impl RuntimeEngine {
    /// Create a new runtime engine with all subsystems initialized.
    pub async fn new(
        mode_config: ModeConfig,
        orchestrator: AutonomousOrchestrator,
        symbols: Vec<String>,
        broker_registry: Option<Arc<BrokerRegistry>>,
    ) -> anyhow::Result<Self> {
        let event_bus = EventBus::new(256);
        let risk_manager = Arc::new(RiskManager::new());
        let state = orchestrator.state.clone();
        let world_model = Arc::new(WorldModelEngine::new());
        let portfolio_reasoner = Arc::new(PortfolioReasoner::new(state.clone()));
        let introspector = Arc::new(Introspector::new(state.clone(), event_bus.clone()));
        let goal_manager = GoalManager::new(state);
        let active_learner = Arc::new(ActiveLearner::new(orchestrator.state.clone()));
        let streaming_reasoner = Arc::new(StreamingReasoner::new(orchestrator.state.clone()));

        // Initialize policy cache from disk + seed from historical trades
        let policy_cache = {
            let cache = PolicyCache::from_disk(orchestrator.state.clone());
            cache.seed_from_history().await;
            Arc::new(tokio::sync::Mutex::new(cache))
        };

        Ok(Self {
            mode_config,
            orchestrator,
            event_bus,
            risk_manager,
            introspector,
            goal_manager,
            world_model,
            portfolio_reasoner,
            active_learner,
            streaming_reasoner,
            policy_cache,
            broker_registry,
            symbols,
            cycles_completed: 0,
            total_pnl: 0.0,
            max_drawdown: 0.0,
            start_time: Instant::now(),
        })
    }

    /// Run the engine according to the configured mode.
    /// Get a reference to the event bus (useful for publishing test events).
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    pub async fn run(mut self) -> anyhow::Result<RunSummary> {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn the introspector as a background task
        let introspector = self.introspector.clone();
        let intro_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            introspector.run(intro_shutdown).await;
        });

        match self.mode_config.mode {
            TradingMode::Backtest => self.run_backtest(shutdown_tx).await,
            TradingMode::Validate => self.run_validate(shutdown_tx).await,
            TradingMode::Research => self.run_research(shutdown_tx).await,
            _ => self.run_live_or_paper(shutdown_tx).await,
        }
    }

    /// Live or paper mode — event-driven architecture.
    /// Spawns background price fetcher that publishes `PriceTick` events.
    /// Main loop subscribes to events and reacts reactively (no fixed batch loop).
    async fn run_live_or_paper(
        &mut self,
        shutdown_tx: watch::Sender<bool>,
    ) -> anyhow::Result<RunSummary> {
        tracing::info!(
            "Starting {} mode — event-driven (price fetcher reacts via EventBus)",
            self.mode_config.mode
        );

        let symbols = self.symbols.clone();
        let event_bus = self.event_bus.clone();
        let state = self.orchestrator.state.clone();
        let world_model = self.world_model.clone();
        let cycles = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // ═══════════════════════════════════════════════════════════════
        // TASK 1: Price Fetcher — periodically fetches live prices and
        //          publishes PriceTick events to the EventBus.
        // ═══════════════════════════════════════════════════════════════
        let fetcher_bus = event_bus.clone();
        let fetcher_symbols = symbols.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default();

            let mut sym_idx = 0usize;
            loop {
                let sym = fetcher_symbols[sym_idx % fetcher_symbols.len()].clone();
                sym_idx += 1;

                match api_clients::fetch_live_bar(&client, &sym).await {
                    Ok(bar) if bar.close > 0.0 => {
                        fetcher_bus.publish(AgentEvent::PriceTick {
                            symbol: sym.clone(),
                            price: bar.close,
                            volume: bar.volume,
                            timestamp: bar.timestamp,
                            source: crate::event_bus::PriceSource::Rest,
                        });
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("PriceFetcher API error for {}: {}", sym, e);
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });

        // ═══════════════════════════════════════════════════════════════
        // TASK 2: Periodic Heartbeat — runs portfolio tracking on a 30s timer.
        // ═══════════════════════════════════════════════════════════════
        let hb_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            interval.tick().await;
            loop {
                interval.tick().await;

                // Check for hard stop
                {
                    let portfolio = hb_state.portfolio.read().await;
                    let dd = portfolio.max_drawdown_today.abs() / portfolio.total_equity.max(1.0);
                    if dd * 100.0 > 15.0 {
                        tracing::error!("⚠ HARD STOP: Drawdown {:.1}% exceeds limit.", dd * 100.0);
                    }
                }

                // Log portfolio summary
                let p = hb_state.portfolio.read().await;
                tracing::info!(
                    "[Portfolio] Equity: ₹{:.2} | Positions: {} | P&L: ₹{:.2} | DD: {:.2}%",
                    p.total_equity,
                    p.open_positions.len(),
                    p.daily_pnl,
                    p.max_drawdown_today * 100.0,
                );
            }
        });

        // ═══════════════════════════════════════════════════════════════
        // TASK 3: Periodic Policy Cache Persistence — saves hit/miss
        //          counters and P&L history to disk every 30s so the
        //          orchestrator API sees fresh data.
        // ═══════════════════════════════════════════════════════════════
        let cache_persist = self.policy_cache.clone();
        let pnl_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            interval.tick().await; // skip the immediate first tick
            loop {
                interval.tick().await;
                {
                    let cache = cache_persist.lock().await;
                    // Sample current P&L and equity from portfolio before saving
                    let portfolio = pnl_state.portfolio.read().await;
                    cache.record_pnl_snapshot(portfolio.daily_pnl);
                    cache.record_equity_snapshot(portfolio.total_equity);
                    cache.save();
                }
                tracing::trace!("PolicyCache persisted (periodic 30s save)");
            }
        });

        // ═══════════════════════════════════════════════════════════════
        // MAIN EVENT LOOP: Subscribe to EventBus and react to events.
        // Pipeline runs inline; receiver handles backpressure via broadcast.
        // ═══════════════════════════════════════════════════════════════
        let mut rx = event_bus.subscribe();
        let trades_executed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cache_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ollama_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut ohlcv_tick_counters: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();

        loop {
            match rx.recv().await {
                Ok(event) => {
                    match event {
                        AgentEvent::PriceTick {
                            symbol,
                            price,
                            volume,
                            timestamp,
                            ..
                        } => {
                            if symbol == "HEARTBEAT" {
                                cycles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                continue;
                            }

                            tracing::debug!("⚡ Event: PriceTick {} @ {:.2}", symbol, price);

                            // 1. Update OHLCV history (counter tracks bar rollover at ~1 min intervals)
                            {
                                let mut history = state.ohlcv_history.write().await;
                                let hist = history.entry(symbol.clone()).or_default();
                                let now_rfc = timestamp.to_rfc3339();
                                let tick_count =
                                    ohlcv_tick_counters.entry(symbol.clone()).or_insert(0u32);
                                Self::update_ohlcv_core(hist, price, &now_rfc, tick_count);
                            }

                            // 2. Update world model with real price change magnitude
                            {
                                let hist = state.ohlcv_history.read().await;
                                let prev_close = hist
                                    .get(&symbol)
                                    .and_then(|h| h.iter().rev().nth(1).map(|b| b.close))
                                    .unwrap_or(price);
                                let magnitude =
                                    ((price - prev_close) / prev_close.max(0.001)).abs();
                                world_model.update_belief(
                                    &symbol,
                                    crate::world_model::Evidence::PriceMove {
                                        magnitude,
                                        direction: price.signum(),
                                        volume_confirmation: volume > 0.0,
                                    },
                                );
                            }

                            // 3. Make decision — checks policy cache first, falls back to full pipeline
                            let (_signal, decision_source, executed) =
                                self.make_decision(&symbol, price).await;

                            if executed {
                                trades_executed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }

                            // Track decision source for summary stats
                            match decision_source.as_str() {
                                "cache" => {
                                    cache_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                "ollama" => {
                                    ollama_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                                _ => {
                                    // skipped or error — not a cache hit or Ollama call
                                }
                            }

                            // Log decision source periodically for monitoring
                            if executed
                                || cycles
                                    .load(std::sync::atomic::Ordering::Relaxed)
                                    .is_multiple_of(10)
                            {
                                tracing::debug!(
                                    "Decision for {}: source={}, executed={}",
                                    symbol,
                                    decision_source,
                                    executed
                                );
                            }

                            cycles.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }

                        AgentEvent::Shutdown => {
                            tracing::info!("🛑 Shutdown event received. Exiting.");
                            break;
                        }

                        _ => {}
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("EventBus closed. Exiting event loop.");
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("EventBus lagged — dropped {} events", n);
                    continue;
                }
            }
        }

        // Build summary
        {
            let portfolio = self.orchestrator.state.portfolio.read().await;
            self.total_pnl = portfolio.daily_pnl;
            let dd = portfolio.max_drawdown_today.abs() / portfolio.total_equity.max(1.0);
            self.max_drawdown = dd;
        }
        self.cycles_completed = cycles.load(std::sync::atomic::Ordering::Relaxed);
        let trade_count = trades_executed.load(std::sync::atomic::Ordering::Relaxed);
        let cache_hit_count = cache_hits.load(std::sync::atomic::Ordering::Relaxed);
        let ollama_call_count = ollama_calls.load(std::sync::atomic::Ordering::Relaxed);
        // Signal the introspector to stop (dropping the sender wakes the receiver)
        drop(shutdown_tx);

        Ok(self.build_summary(Some(trade_count), cache_hit_count, ollama_call_count))
    }

    /// Update or create 1m OHLCV bar using tredo_core::OhlcvBar (which uses String timestamps).
    /// Uses count-based rollover via `tick_count`: every 12 updates (≈1 minute at 5s polling),
    /// starts a new bar. The tick_counter is stored separately so `volume` retains its real meaning.
    fn update_ohlcv_core(
        history: &mut Vec<tredo_core::OhlcvBar>,
        price: f64,
        now_rfc: &str,
        tick_count: &mut u32,
    ) {
        *tick_count += 1;

        if history.is_empty() {
            history.push(tredo_core::OhlcvBar {
                timestamp: now_rfc.to_string(),
                open: price,
                high: price,
                low: price,
                close: price,
                volume: 0.0,
            });
            return;
        }

        // Roll over after 12 updates (~1 minute at 5s polling)
        if *tick_count >= 12 {
            *tick_count = 0;
            let last_close = history.last().unwrap().close;
            history.push(tredo_core::OhlcvBar {
                timestamp: now_rfc.to_string(),
                open: last_close,
                high: price,
                low: price,
                close: price,
                volume: 0.0,
            });
            if history.len() > 200 {
                history.remove(0);
            }
        } else {
            let last_idx = history.len() - 1;
            let last = &mut history[last_idx];
            if price > last.high {
                last.high = price;
            }
            if price < last.low {
                last.low = price;
            }
            last.close = price;
        }
    }

    /// Backtest mode — runs historical data through the pipeline.
    async fn run_backtest(
        &mut self,
        _shutdown_tx: watch::Sender<bool>,
    ) -> anyhow::Result<RunSummary> {
        tracing::info!("Starting backtest mode");

        let data_path = self
            .mode_config
            .backtest_data_path
            .clone()
            .context("--data <path> required for backtest mode")?;

        let config = crate::data_feed::FeedConfig {
            symbols: self.symbols.clone(),
            interval_secs: 60,
            lookback_bars: 100,
            start: self.mode_config.backtest_start,
            end: self.mode_config.backtest_end,
        };

        let mut feed = BacktestFeed::from_csv(&data_path, config)
            .map_err(|e| anyhow::anyhow!("Failed to load backtest CSV data: {}", e))?;
        tracing::info!("Loaded {} bars from {}", feed.bars().len(), data_path);

        let mut bar_count = 0;
        while feed.has_next() {
            let bars = feed
                .next_bars()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read next bar: {}", e))?;
            for bar in bars {
                bar_count += 1;

                // ═══ FIX: Push bar data into OHLCV history so the pipeline sees real data ═══
                // Without this, the pipeline reads from an empty ohlcv_history and all
                // indicators (RSI, MACD, ATR) return default values, causing HOLD every cycle.
                {
                    let ohlcv_bar = tredo_core::OhlcvBar {
                        timestamp: bar.timestamp.to_rfc3339(),
                        open: bar.open,
                        high: bar.high,
                        low: bar.low,
                        close: bar.close,
                        volume: bar.volume,
                    };
                    let mut history = self.orchestrator.state.ohlcv_history.write().await;
                    for sym in &self.symbols {
                        let hist = history.entry(sym.clone()).or_default();
                        hist.push(ohlcv_bar.clone());
                        if hist.len() > 200 {
                            hist.remove(0);
                        }
                    }
                }

                // Also set the market regime from the accumulated bars after each push
                if bar_count == 30 {
                    // After 30 bars, infer a preliminary regime
                    let history = self.orchestrator.state.ohlcv_history.read().await;
                    for sym in &self.symbols {
                        if let Some(bars) = history.get(sym) {
                            if bars.len() >= 10 {
                                let prices: Vec<f64> = bars.iter().map(|b| b.close).collect();
                                let highs: Vec<f64> = bars.iter().map(|b| b.high).collect();
                                let lows: Vec<f64> = bars.iter().map(|b| b.low).collect();
                                let regime = tredo_autonomous::helpers::estimate_market_regime(
                                    &prices, &highs, &lows,
                                );
                                *self.orchestrator.state.market_regime.write().await = Some(regime);
                                tracing::info!(
                                    "[Backtest] Inferred regime for {}: {:?}",
                                    sym,
                                    regime
                                );
                            }
                        }
                    }
                }

                // Run the pipeline on each symbol
                for sym in &self.symbols {
                    match self.orchestrator.run_full_pipeline(sym).await {
                        Ok(ref summary) => {
                            tracing::info!(
                                "[Backtest] Bar {}/{} | {} | {}",
                                bar_count,
                                feed.bars().len(),
                                sym,
                                summary.reason
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                "Pipeline error at bar {} for {}: {}",
                                bar_count,
                                sym,
                                e
                            );
                        }
                    }
                }

                self.goal_manager.update_progress().await;
                self.cycles_completed += 1;

                // Update P&L tracking
                let portfolio = self.orchestrator.state.portfolio.read().await;
                let dd = portfolio.max_drawdown_today.abs() / portfolio.total_equity.max(1.0);
                self.max_drawdown = self.max_drawdown.max(dd);

                // Small delay between bars to avoid overwhelming Ollama
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }

            if bar_count % 25 == 0 {
                tracing::info!(
                    "Backtest progress: {:.1}% ({} bars processed)",
                    feed.progress() * 100.0,
                    bar_count
                );
            }
        }

        tracing::info!(
            "Backtest complete: {} bars processed, {} cycles",
            bar_count,
            self.cycles_completed
        );

        // Final P&L
        {
            let portfolio = self.orchestrator.state.portfolio.read().await;
            self.total_pnl = portfolio.daily_pnl;
        }

        Ok(self.build_summary(None, 0, 0))
    }

    /// Validate mode — extended validation with self-evolution tracking.
    /// If a data path is provided, loads historical OHLCV data from CSV.
    /// When induce_regret is true, forces tight stop-losses to generate
    /// high-regret episodes that trigger MetaControl rule adaptation.
    async fn run_validate(
        &mut self,
        _shutdown_tx: watch::Sender<bool>,
    ) -> anyhow::Result<RunSummary> {
        tracing::info!(
            "Starting validate mode ({} cycles, induce_regret={})",
            self.mode_config.validate_cycles,
            self.mode_config.induce_regret
        );

        // ── Load data into OHLCV history if a data path was provided ────────
        if let Some(ref data_path) = self.mode_config.backtest_data_path {
            let config = crate::data_feed::FeedConfig {
                symbols: self.symbols.clone(),
                interval_secs: 300, // 5m
                lookback_bars: 100,
                start: self.mode_config.backtest_start,
                end: self.mode_config.backtest_end,
            };
            match crate::backtest_feed::BacktestFeed::from_csv(data_path, config) {
                Ok(feed) => {
                    let bars = feed.bars();
                    tracing::info!(
                        "Loaded {} bars from {} for validate mode",
                        bars.len(),
                        data_path
                    );
                    for (i, bar) in bars.iter().enumerate() {
                        let ohlcv_bar = tredo_core::OhlcvBar {
                            timestamp: bar.timestamp.to_rfc3339(),
                            open: bar.open,
                            high: bar.high,
                            low: bar.low,
                            close: bar.close,
                            volume: bar.volume,
                        };
                        // ⚠️ MUST drop the write lock before acquiring the read lock
                        // to avoid a deadlock (tokio RwLock is not reentrant)
                        {
                            let mut history = self.orchestrator.state.ohlcv_history.write().await;
                            for sym in &self.symbols {
                                let hist = history.entry(sym.clone()).or_default();
                                hist.push(ohlcv_bar.clone());
                                if hist.len() > 200 {
                                    hist.remove(0);
                                }
                            }
                        } // write lock dropped here

                        // Infer regime at bar 30 (separate scope — no write lock held)
                        if i == 30 {
                            let history = self.orchestrator.state.ohlcv_history.read().await;
                            for sym in &self.symbols {
                                if let Some(bars) = history.get(sym) {
                                    if bars.len() >= 10 {
                                        let prices: Vec<f64> =
                                            bars.iter().map(|b| b.close).collect();
                                        let highs: Vec<f64> = bars.iter().map(|b| b.high).collect();
                                        let lows: Vec<f64> = bars.iter().map(|b| b.low).collect();
                                        let regime =
                                            tredo_autonomous::helpers::estimate_market_regime(
                                                &prices, &highs, &lows,
                                            );
                                        *self.orchestrator.state.market_regime.write().await =
                                            Some(regime);
                                        tracing::info!(
                                            "[Validate] Inferred regime for {}: {:?}",
                                            sym,
                                            regime
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Could not load data file {}: {}. Running with empty OHLCV history.",
                        data_path,
                        e
                    );
                }
            }
        } else {
            tracing::warn!("No --data path provided for validate mode. Pipeline will run with empty OHLCV history (likely all HOLD decisions).");
        }

        // ── Apply induced regret if enabled ───────────────────────────────
        if self.mode_config.induce_regret {
            tracing::info!(">>> INDUCING REGRET: tightening stop-loss and reducing position sizing to force trade exits");
            // Tighten rules to force high-regret outcomes
            {
                let mut rules = self.orchestrator.state.rules.write().await;
                // Halve max risk per trade to make positions smaller and tighter
                rules.max_risk_per_trade *= 0.5;
                // Reduce min_confluence_score to allow more trades through
                rules.min_confluence_score = (rules.min_confluence_score * 0.7).max(0.30);
                tracing::info!(
                    "[Validate] Induced regret: max_risk={:.4}, min_confluence={:.2}",
                    rules.max_risk_per_trade,
                    rules.min_confluence_score
                );
            }
        }

        // ── Initialize the OutcomeProcessor for self-evolution tracking ───
        use tredo_autonomous::execution_coordinator::init_outcome_processor;
        let _processor = init_outcome_processor(
            (*self.orchestrator.state.episode_store).clone(),
            (*self.orchestrator.state.episode_store).clone(),
        )
        .await;

        // ── Use SelfEvolutionValidator for structured validation ──────────
        use tredo_autonomous::self_evolution::SelfEvolutionValidator;

        // Create a fresh orchestrator with shared state (the validator needs owned)
        let mut orch_clone = tredo_autonomous::orchestrator_struct::AutonomousOrchestrator::new(
            self.orchestrator.state.clone(),
        );
        orch_clone.init_tredo(); // required so run_full_pipeline doesn't panic

        let symbols: Vec<&str> = self.symbols.iter().map(|s| s.as_str()).collect();

        let validator = SelfEvolutionValidator::new(orch_clone);
        match validator
            .run_extended_validation(
                &symbols,
                self.mode_config.validate_cycles,
                self.mode_config.induce_regret,
            )
            .await
        {
            Ok(report) => {
                println!("\n{}", report.summary());
                self.total_pnl = 0.0;
                self.cycles_completed = report.total_cycles;
            }
            Err(e) => {
                tracing::error!("SelfEvolutionValidator failed: {}", e);
            }
        }

        tracing::info!("Validation complete after {} cycles", self.cycles_completed);
        Ok(self.build_summary(None, 0, 0))
    }

    /// Research mode — observe market without executing any trades.
    /// Uses tokio::select! to listen for shutdown signal while observing.
    async fn run_research(
        &mut self,
        shutdown_tx: watch::Sender<bool>,
    ) -> anyhow::Result<RunSummary> {
        tracing::info!("Starting research mode — observing only, no trades");
        let mut research_rx = self.event_bus.subscribe();

        loop {
            tokio::select! {
                // Listen for shutdown signal
                _ = research_rx.recv() => {
                    tracing::info!("Research mode received shutdown signal — exiting.");
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {
                    // Observe the market via introspection
                    let introspection = self.introspector.introspect().await;

                    // Update world model beliefs
                    for sym in &self.symbols {
                        self.world_model.update_belief(sym, crate::world_model::Evidence::PriceMove {
                            magnitude: 0.001,
                            direction: 0.0,
                            volume_confirmation: true,
                        });
                    }

                    // Update goals
                    self.goal_manager.update_progress().await;

                    if self.cycles_completed.is_multiple_of(50) {
                        tracing::info!(
                            "Research observation {} | Mode: {:?} | Beliefs: {} symbols | Goals: {}",
                            self.cycles_completed,
                            introspection.mode,
                            self.symbols.len(),
                            self.goal_manager.summary(),
                        );
                    }

                    self.cycles_completed += 1;
                }
            }
        }

        drop(shutdown_tx);
        Ok(self.build_summary(None, 0, 0))
    }

    fn build_summary(
        &self,
        trades_executed: Option<usize>,
        cache_hits: usize,
        ollama_calls: usize,
    ) -> RunSummary {
        RunSummary {
            mode: self.mode_config.mode,
            cycles_completed: self.cycles_completed,
            events_processed: self.event_bus.published_count(),
            trades_executed: trades_executed.unwrap_or(0),
            cache_hits,
            ollama_calls,
            total_pnl: self.total_pnl,
            max_drawdown: self.max_drawdown,
            duration_secs: self.start_time.elapsed().as_secs(),
            risk_summary: self.risk_manager.summary(),
            goal_summary: self.goal_manager.summary(),
        }
    }

    // ── Policy Cache Integration ─────────────────────────────────────

    /// Decide whether to trade for a symbol, consulting the policy cache first.
    ///
    /// Returns `(signal, source, executed)` where:
    /// - `signal` — the trade signal (if any)
    /// - `source` — `"cache"` (hit, no Ollama), `"ollama"` (full pipeline), or `"skipped:..."`
    /// - `executed` — whether a trade was actually placed
    ///
    /// For cache hits, execution happens inline. For Ollama fallback,
    /// the full pipeline already handles execution internally.
    pub async fn make_decision(
        &self,
        symbol: &str,
        current_price: f64,
    ) -> (Option<tredo_autonomous::types::TradeSignal>, String, bool) {
        // 1. Check introspection — don't trade if the system is uncertain
        let intro = self.introspector.introspect().await;
        if matches!(intro.mode, crate::introspector::AgentMode::Wait) {
            return (None, "skipped:wait_mode".to_string(), false);
        }

        // 2. Extract features and check the cache
        let features = {
            let cache = self.policy_cache.lock().await;
            cache.extract_features(symbol).await
        };

        let cached = {
            let cache = self.policy_cache.lock().await;
            cache.lookup(&features)
        };

        // Record the lookup result
        {
            let cache = self.policy_cache.lock().await;
            cache.record_lookup(cached.is_some());
        }

        if let Some(entry) = cached {
            tracing::info!(
                "PolicyCache HIT for {}: {:?} (win_rate={:.1}%, samples={}, confidence={:.2})",
                symbol,
                entry.recommended_action,
                entry.win_rate() * 100.0,
                entry.sample_size,
                entry.confidence()
            );

            // Build a signal from the cached decision
            if let Some(signal) = self
                .signal_from_cache(&features, &entry, current_price)
                .await
            {
                // Route execution based on mode
                let is_live = self.mode_config.mode == crate::mode::TradingMode::Live
                    && self.broker_registry.is_some();

                let result = if is_live {
                    self.execute_live_trade(&signal, current_price).await
                } else {
                    self.orchestrator
                        .execution
                        .execute_paper_trade(&signal)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                };

                match result {
                    Ok(_) => {
                        tracing::info!(
                            "✅ PolicyCache: trade executed for {} (cached, {})",
                            symbol,
                            if is_live { "live" } else { "paper" }
                        );
                        return (Some(signal), "cache".to_string(), true);
                    }
                    Err(e) => {
                        tracing::warn!("PolicyCache: execution failed for {}: {}", symbol, e);
                        return (Some(signal), "cache_exec_failed".to_string(), false);
                    }
                }
            }
            return (None, "cache:no_signal".to_string(), false);
        }

        // 3. Fall back to Ollama via the existing full pipeline
        match self.orchestrator.run_full_pipeline(symbol).await {
            Ok(summary) => {
                let executed = summary.executed;
                let source = if summary.final_signal.is_some() {
                    "ollama"
                } else {
                    "skipped:no_signal"
                };
                (summary.final_signal, source.to_string(), executed)
            }
            Err(e) => {
                tracing::error!("Full pipeline failed for {}: {}", symbol, e);
                (None, format!("error:{}", e), false)
            }
        }
    }

    /// Execute a live trade through the registered broker adapter.
    ///
    /// Converts the `TradeSignal` into an `OrderRequest` and places it via
    /// the active broker from `BrokerRegistry`. Also tracks the position in
    /// the local portfolio manager state for consistency.
    async fn execute_live_trade(
        &self,
        signal: &tredo_autonomous::types::TradeSignal,
        market_price: f64,
    ) -> Result<(), String> {
        let registry = self
            .broker_registry
            .as_ref()
            .ok_or_else(|| "No broker registry configured for live mode".to_string())?;

        let broker = registry.active_broker().await;

        // Convert TradeSignal → OrderRequest
        let request = OrderRequest {
            symbol: signal.symbol.clone(),
            direction: signal.direction,
            order_type: OrderType::Market,
            qty: signal.position_size.round().max(1.0) as i32,
            price: None,
            stop_loss: Some(signal.stop_loss),
            take_profit: Some(signal.take_profit),
            strategy: Some("policy_cache".to_string()),
            client_order_id: Some(format!("cache-{}", chrono::Utc::now().timestamp_millis())),
        };

        tracing::info!(
            "[LiveBroker] Placing {} {} qty={} @ {:.2} (SL={:.2} TP={:.2})",
            request.symbol,
            if request.direction == TradeDirection::Long {
                "BUY"
            } else {
                "SELL"
            },
            request.qty,
            market_price,
            request.stop_loss.unwrap_or(0.0),
            request.take_profit.unwrap_or(0.0),
        );

        // Place order through the live broker
        broker.place_order(request, market_price).await?;

        // Also track the position locally via the orchestrator's portfolio manager
        self.orchestrator
            .portfolio
            .add_position(signal)
            .await
            .map_err(|e| e.to_string())?;

        // Push COT entry for visibility
        self.orchestrator
            .state
            .push_cot(
                "LiveBroker",
                &format!(
                    "Execute {} {} @ {:.2}",
                    signal.symbol,
                    if signal.direction == TradeDirection::Long {
                        "BUY"
                    } else {
                        "SELL"
                    },
                    signal.entry_price
                ),
                "FILLED",
                "Live broker execution (cache hit)",
                signal.confidence_score,
                0,
                None,
                Some(signal.symbol.clone()),
            )
            .await;

        Ok(())
    }

    /// Build a trade signal from a cached policy entry (no Ollama call).
    async fn signal_from_cache(
        &self,
        features: &crate::policy_cache::MarketFeatures,
        entry: &crate::policy_cache::PolicyEntry,
        current_price: f64,
    ) -> Option<tredo_autonomous::types::TradeSignal> {
        // Compute SL/TP from volatility bucket
        let atr_pct = (features.volatility_bucket.max(1) as f64) / 100.0;
        let sl = match entry.recommended_action {
            TradeDirection::Long => current_price * (1.0 - atr_pct * 1.5),
            TradeDirection::Short => current_price * (1.0 + atr_pct * 1.5),
        };
        let tp = match entry.recommended_action {
            TradeDirection::Long => current_price * (1.0 + atr_pct * 3.0),
            TradeDirection::Short => current_price * (1.0 - atr_pct * 3.0),
        };

        // Use the orchestrator's execution coordinator for position sizing
        let size = {
            let portfolio = self.orchestrator.state.portfolio.read().await;
            let equity = portfolio.total_equity;
            let risk_amount = equity * 0.02; // 2% risk per trade
            let stop_distance = (current_price - sl).abs();
            if stop_distance > 0.0 {
                (risk_amount / stop_distance)
                    .max(1.0)
                    .min(equity / current_price * 0.1)
            } else {
                10.0
            }
        };

        Some(tredo_autonomous::types::TradeSignal {
            symbol: features.symbol.clone(),
            direction: entry.recommended_action,
            entry_price: current_price,
            stop_loss: sl,
            take_profit: tp,
            position_size: size,
            confidence_score: entry.confidence(),
            confluence_score: 0.5, // not computed for cache hits
            risk_reward_ratio: ((tp - current_price).abs() / (current_price - sl).abs()).max(1.0),
            reasoning: format!(
                "PolicyCache: WR={:.0}% n={} conf={:.2}",
                entry.win_rate() * 100.0,
                entry.sample_size,
                entry.confidence()
            ),
            timestamp: chrono::Utc::now(),
            session_valid: true,
            risk_check_passed: true,
        })
    }

    /// Record the outcome of a trade in the policy cache.
    /// Call this after a position closes with the actual result.
    pub async fn record_outcome(
        &self,
        symbol: &str,
        action: TradeDirection,
        profitable: bool,
        pnl_pct: f64,
        regret: f64,
    ) {
        let features = {
            let cache = self.policy_cache.lock().await;
            cache.extract_features(symbol).await
        };
        let cache = self.policy_cache.lock().await;
        cache.record_outcome(&features, action, profitable, pnl_pct, regret, false);
        cache.save();
    }

    /// Print a report of policy cache health.
    pub async fn print_cache_report(&self) {
        let cache = self.policy_cache.lock().await;
        let total = cache.total_samples();
        let entries = cache.size();
        tracing::info!(
            "PolicyCache: {} entries, {} total samples | config: min_samples={}, min_win_rate={:.0}%",
            entries,
            total,
            cache.config().min_samples,
            cache.config().min_win_rate * 100.0
        );
        let top = cache.top_performers(5, 5);
        for (i, e) in top.iter().enumerate() {
            tracing::info!(
                "  #{}. {} {:?} WR={:.0}% n={} conf={:.2}",
                i + 1,
                e.features.symbol,
                e.recommended_action,
                e.win_rate() * 100.0,
                e.sample_size,
                e.confidence()
            );
        }
    }
}
