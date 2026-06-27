use chrono::Utc;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Semaphore};
use tokio::time::sleep;
use tredo_autonomous::state::SharedState;
use tredo_autonomous::AutonomousOrchestrator;
use tredo_core::episode::{MarketStateSnapshot, ReasoningStep, TradingEpisode};
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, Agent, MarketContext, OhlcvBar, PivotMethod,
};
use tredo_eventbus::{subjects as event_subjects, EventBus, TredoEvent};
use tracing::{info, warn};

/// Read a loop cadence (in seconds) from an env var, falling back to `default`.
///
/// This lets the self-evolution / engineering loop be exercised on demand during
/// observation and validation runs instead of being pinned to its production
/// cadence. Invalid or zero values fall back to the default.
///
///   TREDO_FAST_LOOP_SECS   (default 5)
///   TREDO_MEDIUM_LOOP_SECS (default 30)
///   TREDO_SLOW_LOOP_SECS   (default 86400 = 24h)
fn loop_cadence_secs(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default)
}

// ── Fast Loop (every 5s): tactical execution, SL/TP, price refresh ─────────

pub async fn fast_loop(
    orchestrator: AutonomousOrchestrator,
    client: reqwest::Client,
    _assets: Vec<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    bus: Arc<dyn EventBus>,
) {
    info!("FastLoop started (5s cadence)");

    // Rate limiter: max 5 concurrent Binance API calls
    let rate_limiter = Arc::new(tokio::sync::Semaphore::new(5));

    loop {
        let now = Utc::now();
        let assets = orchestrator.state.watchlist.read().await.clone();
        let sem = rate_limiter.clone();

        // Parallel price fetching with rate limiting (max 5 concurrent)
        let mut handles = Vec::with_capacity(assets.len());
        for symbol in &assets {
            let sym = symbol.clone();
            let sym_is_crypto = is_crypto_symbol(&sym);
            let cl = client.clone();
            let sem_clone = sem.clone();
            let orch_clone = orchestrator.clone();
            let now_clone = now;
            let bus_clone = bus.clone();

            let handle = tokio::spawn(async move {
                // Get latest known price for fallback
                let old_price = {
                    let portfolio = orch_clone.state.portfolio.read().await;
                    if let Some(pos) = portfolio.open_positions.iter().find(|pos| pos.symbol == sym) {
                        pos.current_price
                    } else {
                        let history = orch_clone.state.ohlcv_history.read().await;
                        history.get(sym.as_str())
                            .and_then(|h| h.last().map(|b| b.close))
                            .unwrap_or(20000.0)
                    }
                };

                // Fetch live price (rate-limited: permit scoped to API call only)
                let price = {
                    let _permit = sem_clone.acquire().await.expect("semaphore");
                    fetch_price(&cl, &sym, sym_is_crypto).await
                }.unwrap_or_else(|e| {
                    warn!(symbol = %sym, error = %e, "API error, using drift price");
                    let drift = ((Utc::now().timestamp_micros() % 2000) as f64 - 1000.0) / 1_000_000.0;
                    old_price * (1.0 + drift)
                });

                // Update P&L for open positions
                let _ = orch_clone.portfolio.update_position_pnl(&sym, price).await;

                // Broadcast price update via event bus
                let _ = bus_clone.publish(
                    &event_subjects::market_price(&sym),
                    &TredoEvent::MarketPrice(tredo_eventbus::MarketPriceEvent {
                        symbol: sym.clone(),
                        price,
                        exchange: if sym_is_crypto { "binance".into() } else { "yahoo".into() },
                        timestamp_micros: chrono::Utc::now().timestamp_micros(),
                    }),
                ).await;

                // Broadcast price update via WebSocket
                let price_update = serde_json::json!({
                    "type": "price",
                    "symbol": sym,
                    "price": price,
                }).to_string();
                let _ = orch_clone.state.update_tx.send(price_update);

                // Update 1m OHLCV
                {
                    let mut history = orch_clone.state.ohlcv_history.write().await;
                    let hist = history.entry(sym.clone()).or_default();
                    update_ohlcv_history(hist, price, &now_clone);
                }
            });
            handles.push(handle);
        }
        // Wait for all price fetches to complete
        for handle in handles {
            let _ = handle.await;
        }

        // SL / TP monitoring & auto-exit
        let _ = orchestrator.execution.run(None).await;

        // Push live portfolio to TUI whenever positions are open (P&L marks update every fast tick)
        {
            let has_positions = orchestrator
                .state
                .portfolio
                .read()
                .await
                .open_positions
                .is_empty();
            if !has_positions {
                orchestrator.state.broadcast_portfolio_snapshot().await;
            }
        }

        // Portfolio snapshot and broadcast every 12 cycles (~1 min)
        let cycle_num = Utc::now().timestamp();
        if cycle_num % 60 < 6 {
            let p = orchestrator.state.portfolio.read().await;
            let mut portfolio_update =
                tredo_autonomous::state::SharedState::portfolio_snapshot_json(&p);
            if let Some(obj) = portfolio_update.as_object_mut() {
                obj.insert("type".to_string(), serde_json::json!("portfolio"));
            }
            let portfolio_update = portfolio_update.to_string();

            // Publish portfolio snapshot via event bus
            let port_snapshot = TredoEvent::PortfolioSnapshot(tredo_eventbus::PortfolioSnapshotEvent {
                total_equity: p.total_equity,
                cash_balance: p.cash_balance,
                daily_pnl: p.daily_pnl,
                open_positions_count: p.open_positions.len() as u32,
                total_trades_today: p.total_trades_today,
                winning_trades_today: p.winning_trades_today,
                consecutive_losses: p.consecutive_losses,
                timestamp_micros: chrono::Utc::now().timestamp_micros(),
            });
            let _ = bus.publish(&event_subjects::portfolio_snapshot(), &port_snapshot).await;

            // Publish health event via event bus
            let health = TredoEvent::Health(tredo_eventbus::HealthEvent {
                service: "orchestrator".to_string(),
                healthy: true,
                latency_ms: None,
                error_message: None,
                timestamp_micros: chrono::Utc::now().timestamp_micros(),
            });
            let _ = bus.publish(&event_subjects::health("orchestrator"), &health).await;

            let _ = orchestrator.state.update_tx.send(portfolio_update);
            log_portfolio_snapshot(&p, &orchestrator.state).await;
        }

        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("FastLoop shutdown signal received. Exiting.");
                break;
            }
            _ = sleep(Duration::from_secs(loop_cadence_secs("TREDO_FAST_LOOP_SECS", 5))) => {}
        }
    }
}

pub async fn medium_loop(
    orchestrator: AutonomousOrchestrator,
    client: reqwest::Client,
    _assets: Vec<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    bus: Arc<dyn EventBus>,
) {
    info!("MediumLoop started (30s cadence — accelerated for observation)");

    loop {
        let now = Utc::now();
        let assets = orchestrator.state.watchlist.read().await.clone();

        // Execute due agent tasks (market_scan, goal_review, etc.)
        execute_due_tasks(&orchestrator, &now).await;

        // ═══ STEP 1: Compute MarketMetrics FIRST (before pipeline) ═══
        let mut metrics_handles = Vec::new();
        for symbol in assets.clone() {
            let sym = symbol.clone();
            let st = orchestrator.state.clone();
            let handle = tokio::spawn(async move {
                let price = {
                    let portfolio = st.portfolio.read().await;
                    if let Some(pos) = portfolio.open_positions.iter().find(|pos| pos.symbol == *sym) {
                        pos.current_price
                    } else {
                        let history = st.ohlcv_history.read().await;
                        history.get(sym.as_str())
                            .and_then(|h| h.last().map(|b| b.close))
                            .unwrap_or(0.0)
                    }
                };
                if price > 0.0 {
                    let meter = tredo_autonomous::market_metrics_meter::MarketMetricsMeter::new(st.clone());
                    let snap = meter.compute_and_store(&sym, price).await;

                    let inferred_regime = match snap.regime_hint.as_str() {
                        "trending_bull" => Some(tredo_autonomous::types::MarketRegime::TrendingBull),
                        "trending_bear" => Some(tredo_autonomous::types::MarketRegime::TrendingBear),
                        "volatile" => Some(tredo_autonomous::types::MarketRegime::Volatile),
                        _ => None,
                    };
                    if let Some(regime) = inferred_regime {
                        let current = *st.market_regime.read().await;
                        if current != Some(regime) {
                            info!(regime = ?regime, hint = %snap.regime_hint, "Setting market regime");
                            *st.market_regime.write().await = Some(regime);
                        }
                    }

                    info!(symbol = %sym, rsi = snap.rsi_14, macd = snap.macd_hist, atr_pct = snap.atr_pct * 100.0, hint = %snap.regime_hint, conf = snap.confluence_hint, "Metrics computed");
                }
            });
            metrics_handles.push(handle);
        }
        for handle in metrics_handles {
            let _ = handle.await;
        }

        // ═══ STEP 1b: Refresh real OHLCV klines from Binance/Yahoo ═══
        // Skip if existing bars are fresh (< 90s old) — the fast loop already
        // keeps 1m OHLCV up-to-date, so we avoid redundant API calls.
        let ohlcv_limiter = Arc::new(Semaphore::new(3));
        let mut ohlcv_handles = Vec::new();
        for symbol in assets.clone() {
            let sym = symbol.clone();
            let cl = client.clone();
            let st = orchestrator.state.clone();
            let sem = ohlcv_limiter.clone();
            ohlcv_handles.push(tokio::spawn(async move {
                // Check if existing OHLCV is fresh enough to skip re-fetch
                let needs_refresh = {
                    let history = st.ohlcv_history.read().await;
                    match history.get(sym.as_str()) {
                        Some(bars) if !bars.is_empty() => {
                            if let Some(last) = bars.last() {
                                let last_ts = chrono::DateTime::parse_from_rfc3339(&last.timestamp)
                                    .map(|dt| dt.with_timezone(&chrono::Utc))
                                    .unwrap_or(chrono::Utc::now());
                                (chrono::Utc::now() - last_ts).num_seconds() > 90
                            } else {
                                true
                            }
                        }
                        _ => true,
                    }
                };
                if !needs_refresh {
                    return Some(());
                }

                let _permit = sem.acquire().await.ok()?;
                let is_crypto = is_crypto_symbol(&sym);
                let bars = if is_crypto {
                    tredo_core::fetch_klines(&cl, &sym, "1m", 100).await.ok()?
                } else {
                    fetch_yahoo_ohlcv(&cl, &sym, "1m", "7d").await.ok()?
                };
                if !bars.is_empty() {
                    st.ohlcv_history.write().await.insert(sym, bars);
                }
                Some(())
            }));
        }
        for handle in ohlcv_handles {
            let _ = handle.await;
        }

        // ═══ STEP 2: Run pipeline SEQUENTIALLY (one symbol at a time) ═══
        // Parallel runs caused LLM contention, portfolio races, and wrong results.
        for symbol in assets.clone() {
            let sym = symbol.clone();
            info!(symbol = %sym, "Agentic pipeline starting");

            let outcome = tredo_autonomous::pipeline_runner::run_single_quiet(
                &orchestrator,
                &client,
                &sym,
                true, // quiet=true: skip per-agent COT for automated runs
            ).await;
            let report = &outcome.report;

            if report.executed {
                info!(symbol = %sym, reason = %report.reason, "Trade EXECUTED");
                if let Some(ref summary) = outcome.summary {
                    capture_trade_episode(&orchestrator, summary).await;
                    if let Some(ref signal) = summary.final_signal {
                        let signal_event = TredoEvent::Signal(tredo_eventbus::SignalEvent {
                            symbol: signal.symbol.clone(),
                            action: if signal.direction == tredo_core::TradeDirection::Long { "BUY".to_string() } else { "SELL".to_string() },
                            entry_price: signal.entry_price,
                            stop_loss: signal.stop_loss,
                            take_profit: signal.take_profit,
                            confidence: signal.confidence_score,
                            reasoning: signal.reasoning.clone(),
                            source: "pipeline".to_string(),
                            timestamp_micros: chrono::Utc::now().timestamp_micros(),
                        });
                        let _ = bus.publish(
                            &event_subjects::signal(&sym),
                            &signal_event,
                        ).await;
                    }
                }
            } else if report.success {
                info!(symbol = %sym, action = %report.action, reason = %report.reason, "Pipeline hold");
            } else {
                warn!(symbol = %sym, error = ?report.error, "Pipeline error");
            }
        }

        // Fetch and summarize news for all symbols
        for symbol in &assets {
            let sym = symbol.clone();
            let c = client.clone();
            let st = orchestrator.state.clone();
            tokio::spawn(async move {
                let fetcher = tredo_core::NewsFetcher::new(c, (*st.config).clone());
                match fetcher.fetch_headlines(&sym).await {
                    Ok(headlines) if !headlines.is_empty() => {
                        let summary = st.llm.summarize_news(&headlines, &sym).await;
                        let ctx = tredo_core::NewsContext {
                            symbol: sym.clone(),
                            headlines,
                            summary,
                            fetched_at: Utc::now(),
                        };
                        st.latest_news.write().await.insert(sym, ctx);
                    }
                    Ok(_) => {}
                    Err(e) => warn!(symbol = %sym, error = %e, "Failed to fetch news"),
                }
            });
        }

        // Recalibrate goals & persist state
        recalibrate_goals(&orchestrator).await;
        save_portfolio_state(&orchestrator.state).await;

        // Periodic multi-timeframe refresh (every 30 cycles = ~15m)
        // This avoids rate limiting and CPU spikes from redundant Yahoo scans.
        static CYCLE_COUNT: AtomicU64 = AtomicU64::new(0);
        let cycle = CYCLE_COUNT.fetch_add(1, Ordering::Relaxed);
        if cycle > 0 && cycle % 30 == 0 {
            info!("Scheduled 15-minute multi-timeframe data refresh...");
            refresh_multi_tf(&assets, &client, &orchestrator.state).await;
        }

        // Log portfolio state
        log_portfolio_snapshot_full(&orchestrator).await;

        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("MediumLoop shutdown signal received. Exiting.");
                break;
            }
            _ = sleep(Duration::from_secs(loop_cadence_secs("TREDO_MEDIUM_LOOP_SECS", 30))) => {}
        }
    }
}

// ── Slow Loop (every 24h): deep reflection, meta-review ────────────────────

pub async fn slow_loop(
    orchestrator: AutonomousOrchestrator,
    state: SharedState,
    mut shutdown_rx: watch::Receiver<bool>,
    _bus: Arc<dyn EventBus>,
) {
    let slow_secs = loop_cadence_secs("TREDO_SLOW_LOOP_SECS", 86400);
    info!(cadence_secs = slow_secs, "SlowLoop started — deep reflection + meta-control (engineering loop)");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                info!("SlowLoop shutdown signal received. Exiting.");
                break;
            }
            _ = sleep(Duration::from_secs(slow_secs)) => {
                // 0. Rebuild knowledge graph from all closed episodes (GraphRAG)
                info!("Rebuilding knowledge graph...");
                state.rebuild_knowledge_graph().await;
                {
                    let kg = state.knowledge_graph.read().await;
                    info!(nodes = kg.node_count(), edges = kg.edge_count(), "Knowledge graph rebuilt");
                }

                // 1. Run deep reflection on all recent episodes with outcomes
                let since_ts = (Utc::now() - chrono::Duration::days(2)).timestamp();
                let stored = state.memory.load_episodes_since(since_ts).unwrap_or_default();
                info!(count = stored.len(), "Reviewing recent episodes...");

                let mut reflected = 0;
                for (ep_id, json) in &stored {
                    if let Ok(mut episode) = serde_json::from_str::<TradingEpisode>(json) {
                        if episode.outcome.is_some() && episode.reflection.is_none() {
                            let reflection = orchestrator.reflector
                                .deep_reflect_on_episode(&episode, &state.llm)
                                .await
                                .unwrap_or_else(|e| tredo_core::PostTradeReflection {
                                    timestamp: Utc::now(),
                                    lesson: format!("Reflection failed: {e}"),
                                    violated_assumptions: vec![],
                                    regret_score: 0.5,
                                    what_went_wrong: vec![],
                                    what_went_right: vec![],
                                    suggested_rule_change: None,
                                    should_alert: false,
                                });
                            episode.reflection = Some(reflection);
                            if let Ok(updated_json) = serde_json::to_string(&episode) {
                                let _ = state.memory.store_episode(ep_id, &updated_json);
                            }
                            reflected += 1;
                        }
                    }
                }

                if reflected > 0 {
                    info!(count = reflected, "Deep reflection completed");
                }

                // 2. Run meta-control
                let meta = tredo_autonomous::meta_control::MetaControlAgent::new(state.clone());
                match meta.weekly_review(7).await {
                    Ok(report) => {
                        info!(episodes = report.total_episodes_reviewed, high_regret = report.high_regret_episodes, changes = report.changes_applied, "Meta-review completed");
                    }
                    Err(e) => warn!(error = %e, "Meta-review failed"),
                }

                let recent_regrets = state.episode_store
                    .fetch_recent_regret_scores(20)
                    .unwrap_or_default();
                if recent_regrets.len() >= 15 {
                    let avg_regret: f64 = recent_regrets.iter().sum::<f64>() / recent_regrets.len() as f64;
                    if avg_regret > 0.65 {
                        warn!(avg_regret = avg_regret, "High avg regret detected — running EvolvedMetaControl");

                        let meta_evolved = tredo_autonomous::meta_control::EvolvedMetaControl::new(
                            (*state.episode_store).clone(), 0.05, 1,
                        );
                        let current_config = tredo_autonomous::risk_guardian::RiskGuardianConfig::default_fallback();

                        let current_regime = {
                            let regime = state.market_regime.read().await;
                            match *regime {
                                Some(tredo_autonomous::types::MarketRegime::TrendingBull) =>
                                    tredo_autonomous::regime_classifier::MarketRegime::TrendingBull,
                                Some(tredo_autonomous::types::MarketRegime::TrendingBear) =>
                                    tredo_autonomous::regime_classifier::MarketRegime::TrendingBear,
                                Some(tredo_autonomous::types::MarketRegime::Ranging) =>
                                    tredo_autonomous::regime_classifier::MarketRegime::Ranging,
                                Some(tredo_autonomous::types::MarketRegime::Volatile) =>
                                    tredo_autonomous::regime_classifier::MarketRegime::Volatile,
                                _ => tredo_autonomous::regime_classifier::MarketRegime::Ranging,
                            }
                        };

                        match meta_evolved.evaluate_and_adapt(&current_config, current_regime, 10) {
                            Some(new_config) => {
                                info!(new_risk = new_config.max_risk_per_trade_pct, old_risk = current_config.max_risk_per_trade_pct, new_leverage = new_config.absolute_max_leverage, "MetaControl ADAPTED");
                                let _ = state.push_cot("MetaControl", "Slow loop rule adaptation", "RULE_ADAPT",
                                    &format!("Adapted: max_risk {:.4}→{:.4}, leverage {}→{}",
                                        current_config.max_risk_per_trade_pct, new_config.max_risk_per_trade_pct,
                                        current_config.absolute_max_leverage, new_config.absolute_max_leverage),
                                    0.9, 0, None, None).await;
                            }
                            None => {
                                info!("MetaControl: no adaptation needed");
                            }
                        }
                    } else {
                        info!(avg_regret = avg_regret, "Avg regret below threshold, no adaptation");
                    }
                }

                // Check for RULE_REVERT
                if recent_regrets.len() >= 15 {
                    let meta_evolved = tredo_autonomous::meta_control::EvolvedMetaControl::new(
                        (*state.episode_store).clone(), 0.05, 1,
                    );
                    let current_config = tredo_autonomous::risk_guardian::RiskGuardianConfig::default_fallback();
                    if let Some(_restored_config) = meta_evolved.check_and_revert_if_degraded(&current_config) {
                        info!("RULE_REVERT performed");
                        let _ = state.push_cot("MetaControl", "Rule revert due to degradation", "RULE_REVERT",
                            "Performance degraded, reverting to previous rule version", 0.85, 0, None, None).await;
                    }
                }

                // 3. Update agent market summary
                let p = state.portfolio.read().await;
                let summary = format!(
                    "End of day: P&L {:+.2} | {} trades | {} wins / {} losses | Equity: ₹{:.2}",
                    p.daily_pnl, p.total_trades_today, p.winning_trades_today, p.losing_trades_today, p.total_equity
                );
                drop(p);
                let mut market_summary = state.agent_market_summary.write().await;
                *market_summary = summary.clone();
                info!(summary = %summary, "Agent market summary updated");
            }
        }
    }
}

// ── Episode Capture ─────────────────────────────────────────────────────────

async fn capture_trade_episode(
    orchestrator: &AutonomousOrchestrator,
    summary: &tredo_autonomous::types::PipelineSummary,
) {
    if let Some(ref signal) = summary.final_signal {
        let now = Utc::now();
        let ep_id = format!("ep/{}/{}", signal.symbol, now.timestamp());

        let regime = orchestrator.state.market_regime.read().await;
        let regime_str = match *regime {
            Some(tredo_autonomous::types::MarketRegime::TrendingBull) => "TrendingBull",
            Some(tredo_autonomous::types::MarketRegime::TrendingBear) => "TrendingBear",
            Some(tredo_autonomous::types::MarketRegime::Ranging) => "Ranging",
            Some(tredo_autonomous::types::MarketRegime::Volatile) => "Volatile",
            Some(tredo_autonomous::types::MarketRegime::LowLiquidity) => "LowLiquidity",
            None => "Unknown",
        };
        drop(regime);

        let goals = orchestrator.state.trading_goals.read().await;
        let mode_str = format!("{:?}", goals.mode);
        drop(goals);

        let mtf_summary = {
            let mtf = orchestrator.state.multi_timeframe_data.read().await;
            match mtf.get(&signal.symbol) {
                Some(tf_data) => tf_data
                    .iter()
                    .map(|tf| format!("{}: conf={:.1}%", tf.timeframe, tf.confluence * 100.0))
                    .collect::<Vec<_>>()
                    .join(" | "),
                None => "No MTF data".to_string(),
            }
        };

        let portfolio = orchestrator.state.portfolio.read().await;

        let episode = TradingEpisode {
            episode_id: ep_id.clone(),
            timestamp: now,
            symbol: signal.symbol.clone(),
            market_state: MarketStateSnapshot {
                price: signal.entry_price,
                pivot: 0.0,
                r1: 0.0,
                s1: 0.0,
                confluence: signal.confluence_score,
                trend: "N/A".to_string(),
                volatility_24h: 0.0,
                trend_strength: 0.0,
                regime: regime_str.to_string(),
                session_valid: signal.session_valid,
                calendar_events: vec![],
                patterns: vec![],
                news_headlines: vec![],
                multi_tf_summary: mtf_summary,
                trading_mode: mode_str,
                portfolio_heat: portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.risk_amount)
                    .sum::<f64>()
                    / portfolio.total_equity.max(1.0),
                consecutive_losses: portfolio.consecutive_losses,
                daily_pnl_pct: portfolio.daily_pnl_pct,
            },
            action: if signal.direction == tredo_core::TradeDirection::Long {
                "BUY".to_string()
            } else {
                "SELL".to_string()
            },
            entry_price: signal.entry_price,
            stop_loss: signal.stop_loss,
            take_profit: signal.take_profit,
            confidence: signal.confidence_score,
            reasoning_trace: vec![ReasoningStep {
                agent_name: "StrategyDecisionAgent".to_string(),
                agent_tier: "main".to_string(),
                input_summary: format!(
                    "Market analysis for {} @ {:.2}",
                    signal.symbol, signal.entry_price
                ),
                output_summary: signal.reasoning.clone(),
                confidence: signal.confidence_score,
                duration_ms: 0,
            }],
            outcome: None,
            reflection: None,
        };
        drop(portfolio);

        if let Ok(json) = serde_json::to_string(&episode) {
            let _ = orchestrator.state.memory.store_episode(&ep_id, &json);
            orchestrator
                .state
                .latest_episode
                .write()
                .await
                .insert(signal.symbol.clone(), ep_id.clone());
            info!(episode_id = %ep_id, "Stored episode");

            {
                let mtf_analyses = orchestrator.state.multi_tf_analyses.read().await;
                if let Some(tf_analyses) = mtf_analyses.get(&signal.symbol) {
                    let store = &orchestrator.state.episode_store;
                    for (_tf_label, analysis) in tf_analyses.iter() {
                        let pattern_count = analysis.patterns.len();
                        let _ = store.insert_mtf_snapshot(
                            &ep_id,
                            &analysis.timeframe,
                            analysis.metrics.rsi_14,
                            analysis.metrics.macd_hist,
                            analysis.metrics.atr_pct,
                            analysis.metrics.obv_direction,
                            analysis.metrics.adx,
                            analysis.metrics.cci,
                            analysis.metrics.williams_r,
                            analysis.metrics.vwap_deviation,
                            analysis.metrics.mfi,
                            analysis.metrics.cmf,
                            analysis.metrics.aroon_up,
                            analysis.metrics.aroon_down,
                            analysis.metrics.order_flow,
                            analysis.metrics.funding_rate,
                            analysis.confluence,
                            pattern_count,
                            &analysis.aggregated_direction,
                            analysis.aggregated_conviction,
                        );
                    }
                }
            }

            let summary = episode.market_state.to_summary();
            let store_text = format!("{} {}", summary, signal.reasoning);
            let mut vm = orchestrator.state.vector_memory.write().await;
            if let Err(e) = vm
                .store(
                    &ep_id,
                    &signal.symbol,
                    &store_text,
                    None,
                    &orchestrator.state.llm,
                )
                .await
            {
                warn!(episode_id = %ep_id, error = %e, "Failed to embed episode");
            } else {
                info!(episode_id = %ep_id, dims = vm.len(), "Embedded episode");
            }
        }
    }
}

// ── Re-used helpers (moved from main.rs) ────────────────────────────────────

use tredo_autonomous::state::TimeframeData;
use tredo_autonomous::types::PortfolioState;

pub fn is_crypto_symbol(symbol: &str) -> bool {
    tredo_core::is_crypto_symbol(symbol)
}

async fn fetch_price(
    client: &reqwest::Client,
    symbol: &str,
    is_crypto: bool,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    if is_crypto {
        match tredo_core::fetch_binance_price(client, symbol).await {
            Ok(p) => Ok(p),
            Err(e) => {
                warn!(symbol = %symbol, error = %e, "Binance price failed, trying CoinGecko");
                fetch_coingecko_price(client, symbol).await
            }
        }
    } else {
        fetch_yahoo_price(client, symbol).await
    }
}

pub async fn fetch_binance_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    tredo_core::fetch_binance_price(client, symbol).await
}

pub async fn fetch_kraken_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let kraken_sym = match symbol {
        "BTC" => "XBTUSDT",
        "DOGE" => "XDGUSD",
        other => return Err(format!("Kraken: no mapping for {}", other).into()),
    };
    let url = format!("https://api.kraken.com/0/public/Ticker?pair={}", kraken_sym);
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let result = &resp["result"];
    let pair_data = result
        .as_object()
        .and_then(|m| m.values().next())
        .ok_or("no pair data")?;
    let price_str = pair_data["c"][0].as_str().ok_or("no close price")?;
    Ok(price_str.parse()?)
}

pub async fn fetch_coinbase_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let product = format!("{}-USDT", symbol);
    let url = format!(
        "https://api.coinbase.com/api/v3/brokerage/market/products/{}",
        product
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let price_str = resp["price"].as_str().ok_or("no price field")?;
    Ok(price_str.parse()?)
}

pub async fn fetch_coingecko_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let coin_id = symbol_to_coingecko_id(symbol);
    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
        coin_id
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(8))
        .send()
        .await?
        .json()
        .await?;
    let price = resp[coin_id]["usd"].as_f64().ok_or("no usd price")?;
    Ok(price)
}

pub async fn fetch_binance_24h_ticker(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    tredo_core::fetch_ticker_24hr_raw(client, symbol).await
}

fn symbol_to_coingecko_id(symbol: &str) -> String {
    match symbol {
        "BTC" => "bitcoin".to_string(),
        "ETH" => "ethereum".to_string(),
        "SOL" => "solana".to_string(),
        "BNB" => "binancecoin".to_string(),
        "XRP" => "ripple".to_string(),
        "ADA" => "cardano".to_string(),
        "DOGE" => "dogecoin".to_string(),
        "AVAX" => "avalanche-2".to_string(),
        "MATIC" => "matic-network".to_string(),
        "LINK" => "chainlink".to_string(),
        "DOT" => "polkadot".to_string(),
        "ATOM" => "cosmos".to_string(),
        "LTC" => "litecoin".to_string(),
        "BCH" => "bitcoin-cash".to_string(),
        "UNI" => "uniswap".to_string(),
        "AAVE" => "aave".to_string(),
        "NEAR" => "near".to_string(),
        "ICP" => "internet-computer".to_string(),
        "FIL" => "filecoin".to_string(),
        "APT" => "aptos".to_string(),
        "ARB" => "arbitrum".to_string(),
        "OP" => "optimism".to_string(),
        "SUI" => "sui".to_string(),
        "INJ" => "injective-protocol".to_string(),
        "TIA" => "celestia".to_string(),
        "SEI" => "sei-network".to_string(),
        "PEPE" => "pepe".to_string(),
        "WIF" => "dogwifcoin".to_string(),
        "SHIB" => "shiba-inu".to_string(),
        "TON" => "the-open-network".to_string(),
        "TRX" => "tron".to_string(),
        "XLM" => "stellar".to_string(),
        other => other.to_lowercase(),
    }
}

async fn fetch_yahoo_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let yahoo_symbol = match symbol {
        "NIFTY" => "^NSEI",
        "RELIANCE" => "RELIANCE.NS",
        other => other,
    };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range=1d",
        yahoo_symbol
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        )
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let price = resp["chart"]["result"][0]["meta"]["regularMarketPrice"]
        .as_f64()
        .ok_or("regularMarketPrice field missing")?;
    Ok(price)
}

pub async fn fetch_binance_klines(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<OhlcvBar>, Box<dyn std::error::Error + Send + Sync>> {
    tredo_core::fetch_klines(client, symbol, interval, limit).await
}

pub async fn fetch_yahoo_ohlcv(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    range: &str,
) -> Result<Vec<OhlcvBar>, Box<dyn std::error::Error + Send + Sync>> {
    let yahoo_symbol = match symbol {
        "NIFTY" => "^NSEI",
        "RELIANCE" => "RELIANCE.NS",
        other => other,
    };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}",
        yahoo_symbol, interval, range
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        )
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;

    let result = &resp["chart"]["result"][0];
    let timestamps = result["timestamp"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_i64()).collect::<Vec<_>>())
        .unwrap_or_default();
    let quote = &result["indicators"]["quote"][0];
    let opens: Vec<f64> = quote["open"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let highs: Vec<f64> = quote["high"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let lows: Vec<f64> = quote["low"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let closes: Vec<f64> = quote["close"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let volumes: Vec<f64> = quote["volume"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    let n = timestamps
        .len()
        .min(opens.len())
        .min(highs.len())
        .min(lows.len())
        .min(closes.len())
        .min(volumes.len());
    let mut bars = Vec::with_capacity(n);
    for i in 0..n {
        let dt =
            chrono::DateTime::from_timestamp(timestamps[i], 0).unwrap_or_else(chrono::Utc::now);
        bars.push(OhlcvBar {
            timestamp: dt.to_rfc3339(),
            open: opens[i],
            high: highs[i],
            low: lows[i],
            close: closes[i],
            volume: volumes[i],
        });
    }
    Ok(bars)
}

fn update_ohlcv_history(
    history: &mut Vec<OhlcvBar>,
    price: f64,
    now: &chrono::DateTime<chrono::Utc>,
) {
    if history.is_empty() {
        history.push(OhlcvBar {
            timestamp: now.to_rfc3339(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0.0,
        });
        return;
    }
    let last_idx = history.len() - 1;
    let last_ts = history[last_idx].timestamp.clone();
    let last_close = history[last_idx].close;
    let last_time = chrono::DateTime::parse_from_rfc3339(&last_ts)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or(*now);

    if (*now - last_time).num_seconds() >= 60 {
        history.push(OhlcvBar {
            timestamp: now.to_rfc3339(),
            open: last_close,
            high: price,
            low: price,
            close: price,
            volume: 0.0,
        });
        while history.len() > 200 {
            history.remove(0);
        }
    } else {
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

const ALL_TIMEFRAMES: &[(&str, usize, f64, &str)] = &[
    ("1m", 100, 0.04, "1m"),
    ("5m", 100, 0.06, "5m"),
    ("15m", 100, 0.08, "15m"),
    ("30m", 100, 0.10, "30m"),
    ("1h", 100, 0.12, "1h"),
    ("2h", 100, 0.12, "2h"),
    ("4h", 100, 0.12, "4h"),
    ("8h", 100, 0.10, "8h"),
    ("12h", 100, 0.10, "12h"),
    ("1d", 200, 0.10, "1d"),
    ("1w", 52, 0.06, "1w"),
];

async fn fetch_multi_tf_binance(
    client: &reqwest::Client,
    symbol: &str,
    state: &SharedState,
) -> Result<Vec<TimeframeData>, Box<dyn std::error::Error + Send + Sync>> {
    let equity = { state.portfolio.read().await.total_equity };
    let mut results = Vec::new();
    for (interval, limit, _weight, label) in ALL_TIMEFRAMES {
        match tredo_core::fetch_klines(client, symbol, interval, *limit).await {
            Ok(bars) if !bars.is_empty() => {
                let close_price = bars.last().map(|b| b.close).unwrap_or(0.0);
                let pivots = calculate_pivot_points(
                    close_price * 1.01,
                    close_price * 0.99,
                    close_price * 0.998,
                    PivotMethod::Classic,
                );
                let context = MarketContext {
                    symbol: symbol.to_string(),
                    current_price: close_price,
                    high: close_price * 1.01,
                    low: close_price * 0.99,
                    previous_close: close_price * 0.998,
                    timestamp: Utc::now(),
                    daily_pnl: 0.0,
                    equity,
                    consecutive_losses: 0,
                    is_red_folder_day: false,
                    trend_direction: None,
                };
                let confluence = calculate_confluence_score(&context, &pivots);
                info!(symbol = %symbol, label = %label, bars = bars.len(), pivot = pivots.pivot, conf = confluence * 100.0, "MTF Binance");
                results.push(TimeframeData {
                    timeframe: label.to_string(),
                    ohlcv: bars,
                    pivots: Some(pivots),
                    confluence,
                    last_updated: Utc::now(),
                });
            }
            Err(e) => warn!(symbol = %symbol, label = %label, error = %e, "MTF klines failed"),
            _ => {}
        }
    }
    Ok(results)
}

async fn fetch_multi_tf_yahoo(
    client: &reqwest::Client,
    symbol: &str,
    state: &SharedState,
) -> Result<Vec<TimeframeData>, Box<dyn std::error::Error + Send + Sync>> {
    let equity = { state.portfolio.read().await.total_equity };
    let mut results = Vec::new();

    // Map each of the 11 timeframes to (yahoo_interval, yahoo_range)
    // Yahoo valid intervals: 1m, 2m, 5m, 15m, 30m, 60m, 90m, 1h, 1d, 5d, 1wk, 1mo, 3mo
    // Ranges: 1d, 5d, 7d, 30d, 60d, 90d, 1y, etc.
    let mtf_mapping = &[
        ("1m", "1m", "7d"),
        ("5m", "5m", "7d"),
        ("15m", "15m", "30d"),
        ("30m", "30m", "30d"),
        ("1h", "1h", "60d"),
        ("2h", "1h", "60d"), // Yahoo doesn't support 2h directly, we query 1h and we can use it as a proxy or fetch it
        ("4h", "1h", "60d"), // Proxy with 1h or daily depending on density, let's use 1h/60d
        ("8h", "1h", "60d"),
        ("12h", "1h", "60d"),
        ("1d", "1d", "1y"),
        ("1w", "1wk", "2y"),
    ];

    for &(label, yahoo_interval, yahoo_range) in mtf_mapping {
        match fetch_yahoo_ohlcv(client, symbol, yahoo_interval, yahoo_range).await {
            Ok(bars) if !bars.is_empty() => {
                let close_price = bars.last().map(|b| b.close).unwrap_or(0.0);
                let pivots = calculate_pivot_points(
                    close_price * 1.01,
                    close_price * 0.99,
                    close_price * 0.998,
                    PivotMethod::Classic,
                );
                let context = MarketContext {
                    symbol: symbol.to_string(),
                    current_price: close_price,
                    high: close_price * 1.01,
                    low: close_price * 0.99,
                    previous_close: close_price * 0.998,
                    timestamp: Utc::now(),
                    daily_pnl: 0.0,
                    equity,
                    consecutive_losses: 0,
                    is_red_folder_day: false,
                    trend_direction: None,
                };
                let confluence = calculate_confluence_score(&context, &pivots);
                info!(symbol = %symbol, label = %label, bars = bars.len(), pivot = pivots.pivot, conf = confluence * 100.0, "MTF Yahoo");
                results.push(TimeframeData {
                    timeframe: label.to_string(),
                    ohlcv: bars,
                    pivots: Some(pivots),
                    confluence,
                    last_updated: Utc::now(),
                });
            }
            Err(e) => warn!(symbol = %symbol, label = %label, error = %e, "MTF Yahoo klines failed"),
            _ => {}
        }
    }
    Ok(results)
}

async fn compute_mtf_analysis(symbol: &str, state: &SharedState) {
    let tf_data_map = {
        state
            .multi_timeframe_data
            .read()
            .await
            .get(symbol)
            .cloned()
            .unwrap_or_default()
    };
    if tf_data_map.is_empty() {
        return;
    }
    let current_price = tf_data_map
        .first()
        .and_then(|d| d.ohlcv.last())
        .map(|b| b.close)
        .unwrap_or(0.0);
    if current_price <= 0.0 {
        return;
    }

    let mut tf_analyses = HashMap::new();
    let mut total_weight: f64 = 0.0;
    let mut weighted_signal: f64 = 0.0;
    let mut bullish_count: u32 = 0;
    let mut bearish_count: u32 = 0;

    for tf_data in &tf_data_map {
        let tf_label = &tf_data.timeframe;
        let bars = &tf_data.ohlcv;
        if bars.len() < 20 {
            continue;
        }

        let metrics = tredo_autonomous::market_metrics_meter::MarketMetricsMeter::compute_on_bars(
            bars,
            symbol,
            current_price,
            tf_label,
        );
        let patterns = tredo_core::detect_patterns(bars);

        let mut direction = "neutral";
        let mut conviction = 0.5;
        if metrics.rsi_14 < 35.0 && metrics.macd_hist > 0.0 && metrics.obv_direction > 0.0 {
            direction = "bullish";
            conviction = 0.55 + (35.0 - metrics.rsi_14) / 70.0;
        } else if metrics.rsi_14 > 65.0 && metrics.macd_hist < 0.0 && metrics.obv_direction < 0.0 {
            direction = "bearish";
            conviction = 0.55 + (metrics.rsi_14 - 65.0) / 70.0;
        } else if metrics.rsi_14 < 30.0 {
            direction = "bullish";
            conviction = 0.6;
        } else if metrics.rsi_14 > 70.0 {
            direction = "bearish";
            conviction = 0.6;
        } else if metrics.macd_hist > 0.0 && metrics.aroon_up > 70.0 {
            direction = "bullish";
            conviction = 0.55;
        } else if metrics.macd_hist < 0.0 && metrics.aroon_down > 70.0 {
            direction = "bearish";
            conviction = 0.55;
        }
        conviction = conviction.min(0.95);

        let weight = ALL_TIMEFRAMES
            .iter()
            .find(|(_, _, _, label)| *label == tf_label)
            .map(|(_, _, w, _)| *w)
            .unwrap_or(0.05);

        let analysis = tredo_autonomous::state::TimeframeAnalysis {
            timeframe: tf_label.clone(),
            metrics: metrics.clone(),
            patterns: patterns.clone(),
            confluence: tf_data.confluence,
            aggregated_direction: direction.to_string(),
            aggregated_conviction: conviction,
            last_updated: Utc::now(),
        };
        tf_analyses.insert(tf_label.clone(), analysis);

        let signal = match direction {
            "bullish" => conviction,
            "bearish" => -conviction,
            _ => 0.0,
        };
        weighted_signal += weight * signal;
        total_weight += weight;
        if direction == "bullish" {
            bullish_count += 1;
        } else if direction == "bearish" {
            bearish_count += 1;
        }
    }

    if total_weight <= 0.0 {
        return;
    }
    let aggregate_signal = weighted_signal / total_weight;
    let aggregate_direction = if aggregate_signal > 0.15 {
        "bullish"
    } else if aggregate_signal < -0.15 {
        "bearish"
    } else {
        "neutral"
    };
    let total_tfs = tf_analyses.len() as f64;
    let agreement_pct = if total_tfs > 0.0 {
        let dominant = if bullish_count > bearish_count {
            bullish_count
        } else {
            bearish_count
        };
        dominant as f64 / total_tfs
    } else {
        0.0
    };
    let weighted_confluence =
        tf_data_map.iter().map(|d| d.confluence).sum::<f64>() / tf_data_map.len().max(1) as f64;

    let aggregate = tredo_autonomous::state::MultiTfAggregate {
        symbol: symbol.to_string(),
        tf_analyses: tf_analyses.clone(),
        tf_count: tf_analyses.len(),
        aggregate_signal,
        aggregate_direction: aggregate_direction.to_string(),
        agreement_pct,
        weighted_confluence,
        last_updated: Utc::now(),
    };

    state
        .multi_tf_analyses
        .write()
        .await
        .insert(symbol.to_string(), tf_analyses);
    state
        .multi_tf_aggregate
        .write()
        .await
        .insert(symbol.to_string(), aggregate);
    info!(symbol = %symbol, signal = aggregate_signal, direction = %aggregate_direction, tf_count = tf_data_map.len(), "MTF aggregate");
}

pub async fn refresh_multi_tf(assets: &[String], client: &reqwest::Client, state: &SharedState) {
    let rate_limiter = Arc::new(Semaphore::new(3));
    let mut handles = Vec::with_capacity(assets.len());
    for symbol in assets {
        let is_crypto = is_crypto_symbol(symbol);
        let tf_client = client.clone();
        let tf_orch = state.clone();
        let tf_symbol = symbol.clone();
        let state_clone = state.clone();
        let sem = rate_limiter.clone();
        sleep(Duration::from_millis(200)).await;

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore");
            let tf_data = if is_crypto {
                fetch_multi_tf_binance(&tf_client, &tf_symbol, &state_clone)
                    .await
                    .unwrap_or_default()
            } else {
                fetch_multi_tf_yahoo(&tf_client, &tf_symbol, &state_clone)
                    .await
                    .unwrap_or_default()
            };
            if !tf_data.is_empty() {
                tf_orch
                    .multi_timeframe_data
                    .write()
                    .await
                    .insert(tf_symbol.clone(), tf_data);
                compute_mtf_analysis(&tf_symbol, &tf_orch).await;
            }
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
}

pub async fn update_multi_tf_data(
    client: &reqwest::Client,
    orchestrator: &AutonomousOrchestrator,
    symbol: &str,
    is_crypto: bool,
) {
    let tf_data = if is_crypto {
        fetch_multi_tf_binance(client, symbol, &orchestrator.state)
            .await
            .unwrap_or_default()
    } else {
        fetch_multi_tf_yahoo(client, symbol, &orchestrator.state)
            .await
            .unwrap_or_default()
    };
    if !tf_data.is_empty() {
        orchestrator
            .state
            .multi_timeframe_data
            .write()
            .await
            .insert(symbol.to_string(), tf_data);
    }
}

async fn execute_due_tasks(
    orchestrator: &AutonomousOrchestrator,
    now: &chrono::DateTime<chrono::Utc>,
) {
    let tasks = orchestrator.state.agent_tasks.read().await;
    let due_tasks: Vec<(usize, String)> = tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.should_run(now))
        .map(|(i, t)| (i, t.name.clone()))
        .collect();
    drop(tasks);

    for (idx, name) in due_tasks {            info!(task = %name, "Running scheduled task");
        match name.as_str() {
            "market_scan" => {
                let _ = orchestrator.scanner.scan_watchlist().await;
            }
            "position_monitor" => {
                let _ = orchestrator.execution.run(None).await;
            }
            "portfolio_review" => {
                let p = orchestrator.state.portfolio.read().await;
                info!(pnl = p.daily_pnl, trades = p.total_trades_today, "Portfolio review");
                drop(p);
            }
            "goal_review" => {
                let mut goals = orchestrator.state.trading_goals.write().await;
                let p = orchestrator.state.portfolio.read().await;
                goals.recalculate_mode(p.daily_pnl_pct, p.consecutive_losses, p.total_trades_today);
                info!(mode = ?goals.mode, daily_pnl_pct = p.daily_pnl_pct * 100.0, "Trading goals recalibrated");
                drop(p);
                drop(goals);
            }
            _ => {}
        }
        let mut tasks = orchestrator.state.agent_tasks.write().await;
        if idx < tasks.len() {
            tasks[idx].last_run = Some(*now);
        }
    }
}

async fn recalibrate_goals(orchestrator: &AutonomousOrchestrator) {
    let mut goals = orchestrator.state.trading_goals.write().await;
    let p = orchestrator.state.portfolio.read().await;
    goals.recalculate_mode(p.daily_pnl_pct, p.consecutive_losses, p.total_trades_today);
}

pub async fn save_portfolio_state(state: &SharedState) {
    let portfolio = state.portfolio.read().await;
    if let Ok(json) = serde_json::to_string(&*portfolio) {
        let _ = state.memory.store_state("portfolio/state", &json);
    }
}

async fn log_portfolio_snapshot(portfolio: &PortfolioState, state: &SharedState) {
    let goals = state.trading_goals.read().await;
    info!(
        equity = portfolio.total_equity,
        cash = portfolio.cash_balance,
        positions = portfolio.open_positions.len(),
        pnl = portfolio.daily_pnl,
        drawdown_pct = portfolio.max_drawdown_today * 100.0,
        mode = ?goals.mode,
        target_pct = goals.daily_target_pnl_pct * 100.0,
        current_pct = portfolio.daily_pnl_pct * 100.0,
        trades = portfolio.total_trades_today,
        max_trades = goals.max_daily_trades,
        "Portfolio snapshot"
    );
}

async fn log_portfolio_snapshot_full(orchestrator: &AutonomousOrchestrator) {
    let p = orchestrator.state.portfolio.read().await;
    let goals = orchestrator.state.trading_goals.read().await;
    info!(
        equity = p.total_equity,
        cash = p.cash_balance,
        positions = p.open_positions.len(),
        pnl = p.daily_pnl,
        drawdown_pct = p.max_drawdown_today * 100.0,
        mode = ?goals.mode,
        target_pct = goals.daily_target_pnl_pct * 100.0,
        current_pct = p.daily_pnl_pct * 100.0,
        trades = p.total_trades_today,
        max_trades = goals.max_daily_trades,
        "Portfolio snapshot"
    );
    for pos in &p.open_positions {
        let dir = if pos.direction == tredo_core::TradeDirection::Long { "LONG" } else { "SHORT" };
        info!(
            symbol = %pos.symbol, direction = dir,
            qty = pos.quantity, entry = pos.entry_price, current = pos.current_price,
            pnl = pos.unrealized_pnl,
            "Position"
        );
    }
    drop(goals);
    drop(p);
}
