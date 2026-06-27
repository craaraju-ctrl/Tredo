use crate::outcome_processor::OutcomeProcessor;
use crate::state::SharedState;
use crate::types::TradeSignal;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tredo_core::paper_engine::{OrderRequest, OrderStatus, OrderType, TradingMode};
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier, TradeOutcome, TradingEpisode};

/// Global OutcomeProcessor for self-evolution loop.
/// Initialized once during orchestrator startup, used by all trade closes.
static OUTCOME_PROCESSOR: once_cell::sync::Lazy<tokio::sync::Mutex<Option<Arc<OutcomeProcessor>>>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(None));

/// Initialize the global OutcomeProcessor with the given dependencies.
pub async fn init_outcome_processor(
    episode_store: crate::episode_store::EpisodeStore,
    db_for_meta: crate::episode_store::EpisodeStore,
) -> Arc<OutcomeProcessor> {
    use crate::meta_control::EvolvedMetaControl;
    use crate::weight_tuner::AttributionEngine;

    let weight_tuner = AttributionEngine::new(0.10);
    let meta_control = EvolvedMetaControl::new(db_for_meta, 0.05, 1);
    let processor = Arc::new(OutcomeProcessor::new(
        episode_store,
        weight_tuner,
        meta_control,
    ));

    let mut guard = OUTCOME_PROCESSOR.lock().await;
    *guard = Some(processor.clone());
    println!(
        "[OutcomeProcessor] 🧬 Global OutcomeProcessor initialized — self-evolution loop active"
    );
    processor
}

/// Get a reference to the global OutcomeProcessor if initialized.
pub fn get_outcome_processor() -> Option<Arc<OutcomeProcessor>> {
    OUTCOME_PROCESSOR
        .try_lock()
        .ok()
        .and_then(|guard| guard.clone())
}

pub struct ExecutionCoordinatorAgent {
    pub state: SharedState,
}

impl ExecutionCoordinatorAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    fn validate_signal_geometry(signal: &TradeSignal) -> Result<(), String> {
        if signal.entry_price <= 0.0 || signal.stop_loss <= 0.0 || signal.take_profit <= 0.0 {
            return Err(
                "Invalid price levels — entry, stop-loss, and take-profit must be positive".into(),
            );
        }

        match signal.direction {
            tredo_core::TradeDirection::Long => {
                if signal.stop_loss >= signal.entry_price {
                    return Err(format!(
                        "Invalid long geometry: stop-loss {:.2} must be below entry {:.2}",
                        signal.stop_loss, signal.entry_price
                    ));
                }
                if signal.take_profit <= signal.entry_price {
                    return Err(format!(
                        "Invalid long geometry: take-profit {:.2} must be above entry {:.2}",
                        signal.take_profit, signal.entry_price
                    ));
                }
            }
            tredo_core::TradeDirection::Short => {
                if signal.stop_loss <= signal.entry_price {
                    return Err(format!(
                        "Invalid short geometry: stop-loss {:.2} must be above entry {:.2}",
                        signal.stop_loss, signal.entry_price
                    ));
                }
                if signal.take_profit >= signal.entry_price {
                    return Err(format!(
                        "Invalid short geometry: take-profit {:.2} must be below entry {:.2}",
                        signal.take_profit, signal.entry_price
                    ));
                }
            }
        }

        Ok(())
    }

    async fn push_execution_cot(
        &self,
        cot_chain_id: Option<u64>,
        signal: &TradeSignal,
        agent: &str,
        input: &str,
        action: &str,
        reason: &str,
        confidence: f64,
    ) {
        if let Some(cid) = cot_chain_id {
            self.state
                .add_cot_step(
                    cid,
                    agent,
                    input,
                    action,
                    reason,
                    confidence,
                    Some(signal.symbol.clone()),
                )
                .await;
        } else {
            self.state
                .push_cot(
                    agent,
                    input,
                    action,
                    reason,
                    confidence,
                    0,
                    None,
                    Some(signal.symbol.clone()),
                )
                .await;
        }
    }

    /// Call the compliance gateway to validate a trade proposal before execution.
    /// If COMPLIANCE_URL is not set, the check is skipped (backward compatibility).
    /// If COMPLIANCE_URL IS set and the gateway is unreachable, we BLOCK by default (fail-closed).
    async fn compliance_check(
        &self,
        signal: &TradeSignal,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let compliance_url = match std::env::var("COMPLIANCE_URL") {
            Ok(url) if !url.is_empty() => url,
            _ => {
                // No compliance gateway configured — skip check
                println!("[Compliance] ℹ COMPLIANCE_URL not set — compliance checks disabled. Set COMPLIANCE_URL to a tredo-compliance gateway address to enable.");
                return Ok(());
            }
        };

        // Read all portfolio fields in one block to avoid multiple lock acquisitions
        let (
            portfolio_equity,
            heat,
            sym_exposure,
            daily_pnl,
            consecutive_losses,
            trades_today,
            open_positions_count,
            drawdown_pct,
        ) = {
            let portfolio = self.state.portfolio.read().await;
            let eq = portfolio.total_equity;
            let h = if eq > 0.0 {
                portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.risk_amount)
                    .sum::<f64>()
                    / eq
            } else {
                0.0
            };
            let exp = portfolio
                .open_positions
                .iter()
                .filter(|p| p.symbol == signal.symbol)
                .map(|p| p.quantity * p.current_price)
                .sum::<f64>();
            let dd = portfolio.max_drawdown_today / eq.max(1.0);
            (
                eq,
                h,
                exp,
                portfolio.daily_pnl,
                portfolio.consecutive_losses as u32,
                portfolio.total_trades_today,
                portfolio.open_positions.len() as u32,
                dd,
            )
        };

        // Read actual market price from OHLCV history (for price collar check)
        let current_price = {
            let history = self.state.ohlcv_history.read().await;
            history
                .get(&signal.symbol)
                .and_then(|bars| bars.last().map(|b| b.close))
                .unwrap_or(signal.entry_price)
        };

        let proposal = ComplianceProposal {
            symbol: signal.symbol.clone(),
            direction: if signal.direction == tredo_core::TradeDirection::Long {
                "BUY"
            } else {
                "SELL"
            }
            .to_string(),
            entry_price: signal.entry_price,
            stop_loss: signal.stop_loss,
            take_profit: signal.take_profit,
            position_size: signal.position_size,
            position_value: signal.entry_price * signal.position_size,
            leverage: 1,
            confidence_score: signal.confidence_score,
            confluence_score: signal.confluence_score,
            current_price,
            portfolio_equity,
            portfolio_heat: heat,
            daily_pnl,
            daily_pnl_pct: if portfolio_equity > 0.0 {
                daily_pnl / portfolio_equity
            } else {
                0.0
            },
            consecutive_losses,
            open_positions_count,
            trades_today,
            current_drawdown_pct: drawdown_pct * 100.0,
            symbol_exposure: sym_exposure,
            previous_day_volume: 0.0,
            timestamp_micros: Utc::now().timestamp_micros(),
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;

        let resp = client
            .post(format!("{}/check", compliance_url))
            .json(&proposal)
            .send()
            .await
            .map_err(|e| format!("Compliance gateway unreachable (fail-closed): {}", e))?;

        let check: ComplianceResult = resp
            .json()
            .await
            .map_err(|e| format!("Compliance gateway response parse error: {}", e))?;

        if !check.passed {
            let failed: Vec<&str> = check
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.rule_name.as_str())
                .collect();
            return Err(format!(
                "COMPLIANCE BLOCKED: {} (failed rules: {})",
                check.summary,
                failed.join(", ")
            )
            .into());
        }

        Ok(())
    }

    pub async fn execute_paper_trade(
        &self,
        signal: &TradeSignal,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        self.execute_paper_trade_with_chain(signal, None).await
    }

    /// Execute a paper trade, optionally attaching COT entries to a pipeline chain.
    pub async fn execute_paper_trade_with_chain(
        &self,
        signal: &TradeSignal,
        cot_chain_id: Option<u64>,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let direction_str = if signal.direction == tredo_core::TradeDirection::Long {
            "BUY"
        } else {
            "SELL"
        };

        if !signal.risk_check_passed {
            let reason = "Risk check failed — signal not approved for execution";
            self.push_execution_cot(
                cot_chain_id,
                signal,
                "ExecutionCoordinator",
                &format!(
                    "Execute {} {} @ {:.2}",
                    signal.symbol, direction_str, signal.entry_price
                ),
                "REJECTED",
                reason,
                0.0,
            )
            .await;
            return Err(reason.into());
        }

        if let Err(reason) = Self::validate_signal_geometry(signal) {
            self.push_execution_cot(
                cot_chain_id,
                signal,
                "ExecutionCoordinator",
                &format!(
                    "Validate {} {} @ {:.2}",
                    signal.symbol, direction_str, signal.entry_price
                ),
                "REJECTED",
                &reason,
                0.0,
            )
            .await;
            return Err(reason.into());
        }

        // ── Pre-trade compliance check ─────────────────────────────────────
        if let Err(e) = self.compliance_check(signal).await {
            self.push_execution_cot(
                cot_chain_id,
                signal,
                "ComplianceGateway",
                &format!(
                    "Pre-trade compliance for {} {} @ {:.2}",
                    signal.symbol, direction_str, signal.entry_price
                ),
                "BLOCKED",
                &e.to_string(),
                0.0,
            )
            .await;
            return Err(e);
        }

        if signal.position_size <= 0.0 {
            self.push_execution_cot(
                cot_chain_id,
                signal,
                "ExecutionCoordinator",
                &format!(
                    "Execute {} {} @ {:.2}",
                    signal.symbol, direction_str, signal.entry_price
                ),
                "REJECTED",
                "Invalid position size — cannot execute",
                0.0,
            )
            .await;
            return Err("Invalid position size".into());
        }

        // Check current trading mode — route through live broker if LIVE
        let mode = self.state.broker_registry.current_mode().await;

        if mode == TradingMode::Live {
            return self.execute_live_trade(signal).await;
        }

        // ── PAPER MODE (existing logic) ────────────────────────────────────
        let slippage_pct = 0.05;
        let effective_entry = match signal.direction {
            tredo_core::TradeDirection::Long => signal.entry_price * (1.0 + slippage_pct / 100.0),
            tredo_core::TradeDirection::Short => signal.entry_price * (1.0 - slippage_pct / 100.0),
        };
        let effective_slippage = (effective_entry - signal.entry_price).abs();
        let slippage_adj = effective_entry - signal.entry_price;
        let effective_sl = signal.stop_loss + slippage_adj;
        let effective_tp = signal.take_profit + slippage_adj;

        println!(
            "[ExecutionCoordinator] Slippage applied: {:.4} ({:.3}%) → effective entry {:.2}",
            effective_slippage, slippage_pct, effective_entry
        );

        println!(
            "[ExecutionCoordinator] Executing paper trade: {} {} @ {:.2} (was {:.2}) | Qty: {:.4}",
            signal.symbol,
            if signal.direction == tredo_core::TradeDirection::Long {
                "BUY"
            } else {
                "SELL"
            },
            effective_entry,
            signal.entry_price,
            signal.position_size
        );

        let adjusted_signal = TradeSignal {
            symbol: signal.symbol.clone(),
            direction: signal.direction,
            entry_price: effective_entry,
            stop_loss: effective_sl,
            take_profit: effective_tp,
            position_size: signal.position_size,
            confidence_score: signal.confidence_score,
            confluence_score: signal.confluence_score,
            risk_reward_ratio: signal.risk_reward_ratio,
            reasoning: format!("{} | slippage={:.3}%", signal.reasoning, slippage_pct),
            timestamp: signal.timestamp,
            session_valid: signal.session_valid,
            risk_check_passed: signal.risk_check_passed,
        };

        println!("[ExecutionCoordinator] Order filled (simulated)");

        let pm = crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());
        if let Err(e) = pm.add_position(&adjusted_signal).await {
            let reason = e.to_string();
            self.push_execution_cot(
                cot_chain_id,
                &adjusted_signal,
                "ExecutionCoordinator",
                &format!(
                    "Add position {} {} @ {:.2}",
                    adjusted_signal.symbol, direction_str, adjusted_signal.entry_price
                ),
                "REJECTED",
                &reason,
                0.0,
            )
            .await;
            return Err(reason.into());
        }

        let exec_log = format!(
            "EXECUTED: {} {} {:.4} @ {:.2} (slippage-adjusted from {:.2}) | Stop: {:.2} | Target: {:.2}",
            adjusted_signal.symbol,
            direction_str,
            adjusted_signal.position_size,
            adjusted_signal.entry_price,
            signal.entry_price,
            adjusted_signal.stop_loss,
            adjusted_signal.take_profit
        );

        self.push_execution_cot(
            cot_chain_id,
            &adjusted_signal,
            "ExecutionCoordinator",
            &format!(
                "Execute {} {} @ {:.2}",
                adjusted_signal.symbol, direction_str, adjusted_signal.entry_price
            ),
            "EXECUTED",
            &exec_log,
            adjusted_signal.confidence_score,
        )
        .await;

        let _ = self.state.memory.store_decision(
            &format!("execution/{}/{}", signal.symbol, Utc::now().timestamp()),
            &exec_log,
        );

        if let Some(processor) = get_outcome_processor() {
            let snapshot = build_pre_trade_snapshot(&self.state, &adjusted_signal).await;
            processor.register_pending_trade(snapshot).await;
        }

        self.state.broadcast_portfolio_snapshot().await;

        Ok(format!("Paper trade executed: {}", exec_log))
    }

    /// After a successful close_position(), update the corresponding episode's outcome
    /// so the slow loop can reflect on it.
    #[allow(clippy::too_many_arguments)]
    async fn update_episode_outcome(
        &self,
        symbol: &str,
        exit_price: f64,
        exit_reason: &str,
        pnl: f64,
        pos_entry_price: f64,
        pos_quantity: f64,
        pos_entry_time: &chrono::DateTime<Utc>,
    ) {
        // Look up the latest episode for this symbol
        let ep_id = {
            let latest = self.state.latest_episode.read().await;
            latest.get(symbol).cloned()
        };

        if let Some(episode_id) = ep_id {
            // Load the episode from memory
            match self.state.memory.load_episode(&episode_id) {
                Ok(Some(json)) => {
                    if let Ok(mut episode) = serde_json::from_str::<TradingEpisode>(&json) {
                        // Only update if outcome is not already set
                        if episode.outcome.is_none() {
                            let holding_period =
                                (Utc::now() - *pos_entry_time).num_seconds().max(0) as u64;
                            let pnl_pct = if pos_entry_price > 0.0 {
                                pnl / (pos_entry_price * pos_quantity)
                            } else {
                                0.0
                            };

                            episode.outcome = Some(TradeOutcome {
                                exit_price,
                                pnl,
                                pnl_pct,
                                exit_reason: exit_reason.to_string(),
                                holding_period_secs: holding_period,
                                max_unrealized_pnl: pnl.max(0.0),
                                min_unrealized_pnl: pnl.min(0.0),
                                slippage: 0.0,
                            });

                            // Save updated episode back to memory
                            if let Ok(updated_json) = serde_json::to_string(&episode) {
                                let _ = self.state.memory.store_episode(&episode_id, &updated_json);
                                println!("[OutcomeUpdate] 📝 Episode {} updated with outcome: {} {:.2} (P&L: ₹{:.2})",
                                    episode_id, exit_reason, exit_price, pnl);

                                // === Promote vector memory for trained data intelligence ===
                                // Store summary + outcome for semantic recall in future debates/historian/SD.
                                let summary = format!(
                                    "{} {} entry={:.2} exit={} pnl={:.2}% reason={}",
                                    episode.symbol,
                                    episode.action,
                                    episode.entry_price,
                                    exit_reason,
                                    pnl_pct * 100.0,
                                    exit_reason
                                );
                                let regret = episode.reflection.as_ref().map(|r| r.regret_score);
                                let vm = self.state.vector_memory.clone();
                                let llm_clone = self.state.llm.clone();
                                let sym_clone = episode.symbol.clone();
                                tokio::spawn(async move {
                                    let mut vm_write = vm.write().await;
                                    let _ = vm_write
                                        .store(
                                            &episode_id,
                                            &sym_clone,
                                            &summary,
                                            regret,
                                            &llm_clone,
                                        )
                                        .await;
                                });

                                // Wire Notifier for trade outcome (WhatsApp/Telegram alerts)
                                tredo_core::notifier::alert(
                                    &format!("TRADE OUTCOME {}", episode.symbol),
                                    &format!(
                                        "{} {} @ {:.2} -> {} P&L {:.2}%",
                                        episode.action,
                                        episode.entry_price,
                                        exit_price,
                                        exit_reason,
                                        pnl_pct * 100.0
                                    ),
                                )
                                .await;
                            }
                        }
                    }
                }
                Ok(None) => {
                    eprintln!(
                        "[OutcomeUpdate] ⚠ Episode {} not found in memory.",
                        episode_id
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[OutcomeUpdate] ⚠ Failed to load episode {}: {}",
                        episode_id, e
                    );
                }
            }
        }
    }

    /// Refresh all open position prices from the latest OHLCV data.
    /// Without this, positions stall with stale prices and SL/TP never triggers.
    pub async fn refresh_position_prices(&self) {
        let symbols: Vec<String> = {
            let portfolio = self.state.portfolio.read().await;
            portfolio
                .open_positions
                .iter()
                .map(|p| p.symbol.clone())
                .collect()
        };

        if symbols.is_empty() {
            return;
        }

        let history = self.state.ohlcv_history.read().await;
        let mut portfolio = self.state.portfolio.write().await;

        for pos in &mut portfolio.open_positions {
            if let Some(bars) = history.get(&pos.symbol) {
                if let Some(latest) = bars.last() {
                    let old_price = pos.current_price;
                    pos.current_price = latest.close;

                    // Recalculate unrealized P&L
                    pos.unrealized_pnl = match pos.direction {
                        tredo_core::TradeDirection::Long => {
                            (latest.close - pos.entry_price) * pos.quantity
                        }
                        tredo_core::TradeDirection::Short => {
                            (pos.entry_price - latest.close) * pos.quantity
                        }
                    };
                    pos.unrealized_pnl_pct = if pos.entry_price > 0.0 {
                        (pos.unrealized_pnl / (pos.entry_price * pos.quantity)) * 100.0
                    } else {
                        0.0
                    };

                    if (old_price - latest.close).abs() > 0.001 {
                        println!(
                            "[ExecutionCoordinator] 📊 Updated {} price: {:.2} → {:.2} (P&L: ₹{:.2})",
                            pos.symbol, old_price, latest.close, pos.unrealized_pnl
                        );
                    }
                }
            }
        }

        // Recalculate total equity after price updates
        let open_value: f64 = portfolio
            .open_positions
            .iter()
            .map(|p| match p.direction {
                tredo_core::TradeDirection::Long => p.quantity * p.current_price,
                tredo_core::TradeDirection::Short => {
                    (p.quantity * p.entry_price) + p.unrealized_pnl
                }
            })
            .sum();
        portfolio.total_equity = portfolio.cash_balance + open_value;

        if !symbols.is_empty() {
            println!(
                "[ExecutionCoordinator] 🔄 Refreshed prices for {} position(s), equity: ₹{:.2}",
                symbols.len(),
                portfolio.total_equity
            );
        }
    }

    /// Execute a trade through the live broker (Zerodha Kite API).
    /// Includes CircuitBreaker check and LiveOrderManager tracking.
    async fn execute_live_trade(
        &self,
        signal: &TradeSignal,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        // ── Circuit Breaker Check ────────────────────────────────────────
        if !self.state.circuit_breaker.is_trading_allowed().await {
            let reason = self.state.circuit_breaker.halt_reason().await;
            let msg = format!("LIVE TRADE BLOCKED by CircuitBreaker: {:?}", reason);
            eprintln!("[ExecutionCoordinator] {}", msg);
            self.state
                .push_cot(
                    "ExecutionEngine",
                    &format!(
                        "LIVE {} {} @ {:.2}",
                        signal.symbol,
                        if signal.direction == tredo_core::TradeDirection::Long {
                            "BUY"
                        } else {
                            "SELL"
                        },
                        signal.entry_price
                    ),
                    "CIRCUIT_BREAKER_BLOCKED",
                    &msg,
                    0.0,
                    0,
                    None,
                    Some(signal.symbol.clone()),
                )
                .await;
            return Err(msg.into());
        }

        let broker = self.state.broker_registry.active_broker().await;

        // Build a market order request from the trade signal
        let order_req = OrderRequest {
            symbol: signal.symbol.clone(),
            direction: signal.direction,
            order_type: OrderType::Market, // Market order for immediate fill
            qty: signal.position_size.ceil() as i32, // Whole qty for live
            price: Some(signal.entry_price),
            stop_loss: Some(signal.stop_loss),
            take_profit: Some(signal.take_profit),
            strategy: Some("tredo-auto".to_string()),
            client_order_id: None,
        };

        let direction_str = if signal.direction == tredo_core::TradeDirection::Long {
            "BUY"
        } else {
            "SELL"
        };

        println!(
            "[ExecutionCoordinator] LIVE TRADE: {} {} qty={} @ {:.2} SL={:.2} TP={:.2}",
            signal.symbol,
            direction_str,
            order_req.qty,
            signal.entry_price,
            signal.stop_loss,
            signal.take_profit
        );

        match broker
            .place_order(order_req.clone(), signal.entry_price)
            .await
        {
            Ok(order_id) => {
                // ── Register order with LiveOrderManager ─────────────────
                let _ = self
                    .state
                    .live_order_manager
                    .register_order(
                        &order_id,
                        &signal.symbol,
                        signal.direction,
                        order_req.qty,
                        order_req.order_type,
                        order_req.price,
                        order_req.stop_loss,
                        order_req.take_profit,
                        order_req.strategy.clone(),
                    )
                    .await;

                // ── Report fill to CircuitBreaker ─────────────────────────
                self.state.circuit_breaker.report_fill().await;

                let exec_log = format!(
                    "LIVE EXECUTED: {} {} qty={} @ {:.2} | Order: {} | SL: {:.2} | TP: {:.2}",
                    signal.symbol,
                    direction_str,
                    order_req.qty,
                    signal.entry_price,
                    order_id,
                    signal.stop_loss,
                    signal.take_profit
                );

                self.state
                    .push_cot(
                        "ExecutionEngine",
                        &format!(
                            "LIVE {} {} @ {:.2}",
                            signal.symbol, direction_str, signal.entry_price
                        ),
                        "LIVE_FILLED",
                        &exec_log,
                        signal.confidence_score,
                        0,
                        None,
                        Some(signal.symbol.clone()),
                    )
                    .await;

                // Also track in paper portfolio to maintain episode tracking for self-evolution
                let pm = crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());
                let _ = pm.add_position(signal).await;

                if let Some(processor) = get_outcome_processor() {
                    let snapshot = build_pre_trade_snapshot(&self.state, signal).await;
                    processor.register_pending_trade(snapshot).await;
                }

                Ok(format!("Live trade executed: {}", exec_log))
            }
            Err(e) => {
                let err_msg = format!(
                    "LIVE TRADE FAILED: {} {} — {}",
                    signal.symbol, direction_str, e
                );
                eprintln!("[ExecutionCoordinator] {}", err_msg);

                // ── Register rejection with LiveOrderManager ──────────────
                // Use a synthetic order ID since the broker rejected it
                let synth_order_id =
                    format!("REJ-{}-{}", signal.symbol, Utc::now().timestamp_micros());
                let _ = self
                    .state
                    .live_order_manager
                    .register_order(
                        &synth_order_id,
                        &signal.symbol,
                        signal.direction,
                        order_req.qty,
                        order_req.order_type,
                        order_req.price,
                        order_req.stop_loss,
                        order_req.take_profit,
                        order_req.strategy.clone(),
                    )
                    .await;
                let _ = self
                    .state
                    .live_order_manager
                    .update_status(
                        &synth_order_id,
                        OrderStatus::Rejected {
                            reason: e.to_string(),
                        },
                        0,
                        None,
                        Some(e.to_string()),
                    )
                    .await;

                // ── Report rejection to CircuitBreaker ───────────────────
                let rejection_count = self
                    .state
                    .live_order_manager
                    .get_rejection_stats()
                    .await
                    .consecutive_rejections;
                let _ = self
                    .state
                    .circuit_breaker
                    .report_rejection(rejection_count)
                    .await;

                self.state
                    .push_cot(
                        "ExecutionEngine",
                        &format!(
                            "LIVE {} {} @ {:.2}",
                            signal.symbol, direction_str, signal.entry_price
                        ),
                        "LIVE_REJECTED",
                        &err_msg,
                        0.0,
                        0,
                        None,
                        Some(signal.symbol.clone()),
                    )
                    .await;

                Err(err_msg.into())
            }
        }
    }

    async fn check_and_exit_positions(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let mode = self.state.broker_registry.current_mode().await;
        if mode == TradingMode::Live {
            let broker = self.state.broker_registry.active_broker().await;
            let broker_positions = broker.get_positions().await.unwrap_or_default();
            let mut exits = Vec::new();

            let local_positions = {
                let portfolio = self.state.portfolio.read().await;
                portfolio.open_positions.clone()
            };

            let pm = crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());

            for pos in &local_positions {
                // If the position is no longer active on the live broker, it was closed on the exchange!
                let active_on_broker = broker_positions.iter().any(|bp| bp.symbol == pos.symbol);
                if !active_on_broker {
                    println!(
                        "[ExecutionCoordinator] Live position closed on exchange for {}",
                        pos.symbol
                    );

                    let exit_price = pos.current_price;
                    // Determine likely exit reason
                    let exit_reason = if pos.stop_loss > 0.0
                        && (exit_price - pos.stop_loss).abs() < (exit_price - pos.take_profit).abs()
                    {
                        "stop_loss"
                    } else if pos.take_profit > 0.0
                        && (exit_price - pos.take_profit).abs() < (exit_price - pos.stop_loss).abs()
                    {
                        "take_profit"
                    } else {
                        "manual"
                    };

                    match pm.close_position(&pos.symbol, exit_price).await {
                        Ok(pnl) => {
                            self.update_episode_outcome(
                                &pos.symbol,
                                exit_price,
                                exit_reason,
                                pnl,
                                pos.entry_price,
                                pos.quantity,
                                &pos.entry_time,
                            )
                            .await;

                            let episode_id =
                                format!("ep-{}-{}", pos.symbol, pos.entry_time.timestamp());
                            self.spawn_outcome_processing(
                                episode_id,
                                exit_price,
                                pos.symbol.clone(),
                            )
                            .await;

                            // Auto-trigger deep reflection
                            if let Ok(Some(json)) = self.state.memory.load_episode(&pos.symbol) {
                                if let Ok(episode) =
                                    serde_json::from_str::<tredo_core::TradingEpisode>(&json)
                                {
                                    let llm = (*self.state.llm).clone();
                                    let reflector =
                                        crate::reflector::ReflectorAgent::new(self.state.clone());
                                    let _ = reflector.deep_reflect_on_episode(&episode, &llm).await;
                                }
                            }

                            let closed = tredo_core::ClosedTrade {
                                id: format!("{}-{}", pos.symbol, Utc::now().timestamp()),
                                symbol: pos.symbol.clone(),
                                direction: pos.direction,
                                qty: pos.quantity as i32,
                                entry_price: pos.entry_price,
                                exit_price,
                                realized_pnl: pnl,
                                realized_pnl_pct: (pnl / (pos.entry_price * pos.quantity)).abs(),
                                close_reason: match exit_reason {
                                    "stop_loss" => tredo_core::CloseReason::StopLoss,
                                    "take_profit" => tredo_core::CloseReason::TakeProfit,
                                    _ => tredo_core::CloseReason::Manual,
                                },
                                opened_at: pos.entry_time,
                                closed_at: Utc::now(),
                                duration_secs: (Utc::now() - pos.entry_time).num_seconds(),
                                strategy: Some("debate_driven_live".to_string()),
                                order_id: format!("live-{}", Utc::now().timestamp()),
                            };
                            let _ = self.state.memory.store_decision(
                                &format!("closed_trade/{}", closed.id),
                                &serde_json::to_string(&closed).unwrap_or_default(),
                            );

                            self.check_emergency_meta_today().await;

                            exits.push(format!(
                                "{} Closed on Exchange @ {:.2} P&L: ₹{:.2} ({})",
                                pos.symbol, exit_price, pnl, exit_reason
                            ));
                        }
                        Err(e) => {
                            exits.push(format!("{} Live Sync Close FAILED: {}", pos.symbol, e));
                        }
                    }
                } else {
                    // Update current price & P&L from broker
                    if let Some(bp) = broker_positions.iter().find(|bp| bp.symbol == pos.symbol) {
                        let mut portfolio = self.state.portfolio.write().await;
                        if let Some(lp) = portfolio
                            .open_positions
                            .iter_mut()
                            .find(|p| p.symbol == pos.symbol)
                        {
                            lp.current_price = bp.current_price;
                            lp.unrealized_pnl = bp.unrealized_pnl;
                            lp.unrealized_pnl_pct = bp.unrealized_pnl_pct;
                        }
                    }
                }
            }
            return Ok(exits);
        }

        // Refresh position prices from latest OHLCV data so SL/TP can trigger
        self.refresh_position_prices().await;

        let portfolio = self.state.portfolio.read().await;
        let mut exits = Vec::new();
        let positions_snapshot = portfolio.open_positions.clone();
        drop(portfolio);

        let pm = crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());

        for pos in &positions_snapshot {
            let current_price = pos.current_price;
            let entry_price = pos.entry_price;
            let quantity = pos.quantity;
            let entry_time = pos.entry_time;

            let stop_hit = match pos.direction {
                tredo_core::TradeDirection::Long => current_price <= pos.stop_loss,
                tredo_core::TradeDirection::Short => current_price >= pos.stop_loss,
            };

            let tp_hit = match pos.direction {
                tredo_core::TradeDirection::Long => current_price >= pos.take_profit,
                tredo_core::TradeDirection::Short => current_price <= pos.take_profit,
            };

            // ═══ TIME-BASED EXIT (Fix #1) ═══════════════════════════════════
            // If neither SL nor TP has been hit after 4 hours (14400 seconds),
            // close the position at current price to free up locked capital.
            // Prevents "capital death spiral" in range-bound markets.
            // Uses elapsed wall-clock time (not bar count) for accuracy.
            let elapsed_secs = (Utc::now() - pos.entry_time).num_seconds();
            let time_exit = !stop_hit && !tp_hit && elapsed_secs > 14400; // 4 hours

            if stop_hit {
                println!(
                    "[ExecutionCoordinator] STOP LOSS hit for {} @ {:.2}",
                    pos.symbol, current_price
                );
                match pm.close_position(&pos.symbol, pos.stop_loss).await {
                    Ok(pnl) => {
                        self.update_episode_outcome(
                            &pos.symbol,
                            pos.stop_loss,
                            "stop_loss",
                            pnl,
                            entry_price,
                            quantity,
                            &entry_time,
                        )
                        .await;

                        // === NEW: Trigger OutcomeProcessor for self-evolution ===
                        // Use pos.entry_time.timestamp() to match the episode_id registered in execute_paper_trade
                        let exit_price = pos.stop_loss;
                        let episode_id =
                            format!("ep-{}-{}", pos.symbol, pos.entry_time.timestamp());
                        self.spawn_outcome_processing(episode_id, exit_price, pos.symbol.clone())
                            .await;

                        // Auto-trigger deep reflection for self-evolution (trained memory + regret)
                        if let Ok(Some(json)) = self.state.memory.load_episode(&pos.symbol) {
                            if let Ok(episode) =
                                serde_json::from_str::<tredo_core::TradingEpisode>(&json)
                            {
                                let llm = (*self.state.llm).clone();
                                let reflector =
                                    crate::reflector::ReflectorAgent::new(self.state.clone());
                                let _ = reflector.deep_reflect_on_episode(&episode, &llm).await;
                            }
                        }
                        // Record to full journal for better reflection data (self-evolution fuel)
                        let closed = tredo_core::ClosedTrade {
                            id: format!("{}-{}", pos.symbol, Utc::now().timestamp()),
                            symbol: pos.symbol.clone(),
                            direction: pos.direction,
                            qty: pos.quantity as i32,
                            entry_price: pos.entry_price,
                            exit_price: pos.stop_loss,
                            realized_pnl: pnl,
                            realized_pnl_pct: (pnl / (pos.entry_price * pos.quantity)).abs(),
                            close_reason: tredo_core::CloseReason::StopLoss,
                            opened_at: pos.entry_time,
                            closed_at: Utc::now(),
                            duration_secs: (Utc::now() - pos.entry_time).num_seconds(),
                            strategy: Some("debate_driven".to_string()),
                            order_id: format!("paper-{}", Utc::now().timestamp()),
                        };
                        let _ = self.state.memory.store_decision(
                            &format!("closed_trade/{}", closed.id),
                            &serde_json::to_string(&closed).unwrap_or_default(),
                        );
                        // Push adaptation event to COT for observability in TUI (self-evolution visible)
                        let _ = self
                            .state
                            .push_cot(
                                "meta_adapt",
                                &format!("high regret close for {}", pos.symbol),
                                "RULE_TIGHTEN",
                                "max_risk tightened due to SL hit, regret likely high",
                                0.9,
                                0,
                                None,
                                Some(pos.symbol.clone()),
                            )
                            .await;

                        // === NEW: Check for 3+ bad trades today and trigger emergency meta ===
                        self.check_emergency_meta_today().await;

                        exits.push(format!(
                            "{} STOP @ {:.2} P&L: ₹{:.2}",
                            pos.symbol, pos.stop_loss, pnl
                        ));
                    }
                    Err(e) => exits.push(format!("{} STOP FAILED: {}", pos.symbol, e)),
                }
            } else if tp_hit {
                println!(
                    "[ExecutionCoordinator] TAKE PROFIT hit for {} @ {:.2}",
                    pos.symbol, current_price
                );
                match pm.close_position(&pos.symbol, pos.take_profit).await {
                    Ok(pnl) => {
                        self.update_episode_outcome(
                            &pos.symbol,
                            pos.take_profit,
                            "take_profit",
                            pnl,
                            entry_price,
                            quantity,
                            &entry_time,
                        )
                        .await;

                        let exit_price = pos.take_profit;
                        let episode_id =
                            format!("ep-{}-{}", pos.symbol, pos.entry_time.timestamp());
                        self.spawn_outcome_processing(episode_id, exit_price, pos.symbol.clone())
                            .await;

                        if let Ok(Some(json)) = self.state.memory.load_episode(&pos.symbol) {
                            if let Ok(episode) =
                                serde_json::from_str::<tredo_core::TradingEpisode>(&json)
                            {
                                let llm = (*self.state.llm).clone();
                                let reflector =
                                    crate::reflector::ReflectorAgent::new(self.state.clone());
                                let _ = reflector.deep_reflect_on_episode(&episode, &llm).await;
                            }
                        }
                        let closed = tredo_core::ClosedTrade {
                            id: format!("{}-{}", pos.symbol, Utc::now().timestamp()),
                            symbol: pos.symbol.clone(),
                            direction: pos.direction,
                            qty: pos.quantity as i32,
                            entry_price: pos.entry_price,
                            exit_price: pos.take_profit,
                            realized_pnl: pnl,
                            realized_pnl_pct: (pnl / (pos.entry_price * pos.quantity)).abs(),
                            close_reason: tredo_core::CloseReason::TakeProfit,
                            opened_at: pos.entry_time,
                            closed_at: Utc::now(),
                            duration_secs: (Utc::now() - pos.entry_time).num_seconds(),
                            strategy: Some("debate_driven".to_string()),
                            order_id: format!("paper-{}", Utc::now().timestamp()),
                        };
                        let _ = self.state.memory.store_decision(
                            &format!("closed_trade/{}", closed.id),
                            &serde_json::to_string(&closed).unwrap_or_default(),
                        );
                        let _ = self
                            .state
                            .push_cot(
                                "meta_adapt",
                                &format!("TP close for {}", pos.symbol),
                                "PROFIT_LOCK",
                                "successful close, positive signal for memory",
                                0.8,
                                0,
                                None,
                                Some(pos.symbol.clone()),
                            )
                            .await;
                        exits.push(format!(
                            "{} TP @ {:.2} P&L: ₹{:.2}",
                            pos.symbol, pos.take_profit, pnl
                        ));
                    }
                    Err(e) => exits.push(format!("{} TP FAILED: {}", pos.symbol, e)),
                }
            } else if time_exit {
                println!(
                    "[ExecutionCoordinator] ⏰ TIME EXIT for {} @ {:.2} (held {:.0}s, SL={:.2} TP={:.2})",
                    pos.symbol, current_price, elapsed_secs as f64, pos.stop_loss, pos.take_profit
                );
                match pm.close_position(&pos.symbol, current_price).await {
                    Ok(pnl) => {
                        self.update_episode_outcome(
                            &pos.symbol,
                            current_price,
                            "time_exit",
                            pnl,
                            entry_price,
                            quantity,
                            &entry_time,
                        )
                        .await;
                        let episode_id =
                            format!("ep-{}-{}", pos.symbol, pos.entry_time.timestamp());
                        self.spawn_outcome_processing(episode_id, current_price, pos.symbol.clone())
                            .await;
                        let closed = tredo_core::ClosedTrade {
                            id: format!("{}-{}", pos.symbol, Utc::now().timestamp()),
                            symbol: pos.symbol.clone(),
                            direction: pos.direction,
                            qty: pos.quantity as i32,
                            entry_price: pos.entry_price,
                            exit_price: current_price,
                            realized_pnl: pnl,
                            realized_pnl_pct: (pnl / (pos.entry_price * pos.quantity)).abs(),
                            close_reason: tredo_core::CloseReason::Manual,
                            opened_at: pos.entry_time,
                            closed_at: Utc::now(),
                            duration_secs: (Utc::now() - pos.entry_time).num_seconds(),
                            strategy: Some("debate_driven".to_string()),
                            order_id: format!("time-{}", Utc::now().timestamp()),
                        };
                        let _ = self.state.memory.store_decision(
                            &format!("closed_trade/{}", closed.id),
                            &serde_json::to_string(&closed).unwrap_or_default(),
                        );
                        exits.push(format!(
                            "{} TIME EXIT @ {:.2} after {:.0}s | P&L: ₹{:.2}",
                            pos.symbol, current_price, elapsed_secs as f64, pnl
                        ));
                    }
                    Err(e) => exits.push(format!("{} TIME EXIT FAILED: {}", pos.symbol, e)),
                }
            }
        }

        Ok(exits)
    }
}

#[async_trait]
impl Agent for ExecutionCoordinatorAgent {
    fn name(&self) -> &str {
        "ExecutionCoordinatorAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        match input {
            Some(AgentInput::ConfluenceRequest { context }) => {
                let signal = TradeSignal {
                    symbol: context.symbol.clone(),
                    direction: tredo_core::TradeDirection::Long,
                    entry_price: context.current_price,
                    stop_loss: context.current_price * 0.99,
                    take_profit: context.current_price * 1.02,
                    position_size: 10.0,
                    confidence_score: 0.7,
                    confluence_score: 0.7,
                    risk_reward_ratio: 2.0,
                    reasoning: "Auto-generated from execution context".to_string(),
                    timestamp: Utc::now(),
                    session_valid: true,
                    risk_check_passed: true,
                };

                match self.execute_paper_trade(&signal).await {
                    Ok(result) => {
                        println!("[ExecutionCoordinator] {}", result);
                        Ok(AgentOutput::Done)
                    }
                    Err(e) => {
                        println!("[ExecutionCoordinator] Execution failed: {}", e);
                        Ok(AgentOutput::NoOutput)
                    }
                }
            }
            _ => {
                let exits = self.check_and_exit_positions().await?;
                if !exits.is_empty() {
                    for exit in &exits {
                        println!("[ExecutionCoordinator] {}", exit);
                    }
                }
                Ok(AgentOutput::Done)
            }
        }
    }
}

// =====================================================================
// Helper functions for the Self-Evolution loop
// =====================================================================

/// Build a PreTradeSnapshot from current state for the OutcomeProcessor.
async fn build_pre_trade_snapshot(
    state: &SharedState,
    signal: &TradeSignal,
) -> crate::outcome_processor::PreTradeSnapshot {
    // Reconstruct active weights from current DisciplineRules
    let rules = state.rules.read().await;
    let active_weights: HashMap<String, f64> = rules.skill_weights.clone();
    drop(rules);

    // Reconstruct skill predictions from last_skill_votes
    let skill_predictions: HashMap<String, f64> = {
        let votes = state.last_skill_votes.read().await;
        votes
            .iter()
            .map(|v| (v.skill_name.clone(), v.score))
            .collect()
    };

    let direction_str = match signal.direction {
        tredo_core::TradeDirection::Long => "BUY",
        tredo_core::TradeDirection::Short => "SELL",
    };

    let layer_predictions: HashMap<String, f64> = {
        let verdicts = state.last_tri_level_verdict.read().await;
        verdicts
            .get(&signal.symbol)
            .map(crate::tri_level_validator::verdict_to_layer_predictions)
            .unwrap_or_default()
    };

    crate::outcome_processor::PreTradeSnapshot {
        episode_id: format!("ep-{}-{}", signal.symbol, Utc::now().timestamp()),
        symbol: signal.symbol.clone(),
        direction: direction_str.to_string(),
        entry_price: signal.entry_price,
        rule_version: 1,
        active_weights,
        skill_predictions,
        layer_predictions,
    }
}

// =====================================================================
// Compliance Gateway Types — mirror of tredo-compliance's API types
// =====================================================================

/// Trade proposal sent to the compliance gateway for pre-trade validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceProposal {
    pub symbol: String,
    pub direction: String,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub position_size: f64,
    pub position_value: f64,
    pub leverage: u32,
    pub confidence_score: f64,
    pub confluence_score: f64,
    pub current_price: f64,
    pub portfolio_equity: f64,
    pub portfolio_heat: f64,
    pub daily_pnl: f64,
    pub daily_pnl_pct: f64,
    pub consecutive_losses: u32,
    pub open_positions_count: u32,
    pub trades_today: u32,
    pub current_drawdown_pct: f64,
    pub symbol_exposure: f64,
    pub previous_day_volume: f64,
    pub timestamp_micros: i64,
}

/// Response from the compliance gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceResult {
    pub passed: bool,
    pub version: String,
    pub checks: Vec<ComplianceCheckItem>,
    pub summary: String,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceCheckItem {
    pub passed: bool,
    pub rule_name: String,
    pub severity: String,
    pub reason: String,
    pub timestamp_micros: i64,
}

// =====================================================================
// Helper: spawn OutcomeProcessor as a background task (reduces code duplication)
// =====================================================================
impl ExecutionCoordinatorAgent {
    /// Spawn OutcomeProcessor as a background task and apply updated skill weights to DisciplineRules.
    /// UPGRADE 3: Wires WeightTuner output back to DisciplineRules so self-evolution actually changes live behavior.
    async fn spawn_outcome_processing(&self, episode_id: String, exit_price: f64, symbol: String) {
        if let Some(processor) = get_outcome_processor() {
            let current_regime = {
                let regime = self.state.market_regime.read().await;
                match *regime {
                    Some(crate::types::MarketRegime::TrendingBull) => {
                        crate::regime_classifier::MarketRegime::TrendingBull
                    }
                    Some(crate::types::MarketRegime::TrendingBear) => {
                        crate::regime_classifier::MarketRegime::TrendingBear
                    }
                    Some(crate::types::MarketRegime::Ranging) => {
                        crate::regime_classifier::MarketRegime::Ranging
                    }
                    Some(crate::types::MarketRegime::Volatile) => {
                        crate::regime_classifier::MarketRegime::Volatile
                    }
                    _ => crate::regime_classifier::MarketRegime::Ranging,
                }
            };

            let current_config = crate::risk_guardian::RiskGuardianConfig::default_fallback();

            let proc_clone = processor.clone();
            let eid_clone = episode_id.clone();
            let sym_clone = symbol.clone();
            let state_clone = self.state.clone();
            tokio::spawn(async move {
                match proc_clone
                    .process_trade_close(
                        &eid_clone,
                        exit_price,
                        current_regime,
                        10,
                        &current_config,
                        Some(&state_clone),
                    )
                    .await
                {
                    Ok((updated_weights, evolved_config)) => {
                        // === UPGRADE 3: Apply WeightTuner output to DisciplineRules ===
                        // Write the tuned skill weights back so they take effect on the next trade.
                        let mut rules = state_clone.rules.write().await;
                        for (skill_name, weight) in &updated_weights {
                            rules.set_skill_weight(skill_name, *weight);
                        }
                        println!(
                            "[SelfEvolution] ✏️ WeightTuner applied {} updated skill weights to DisciplineRules. Notable: {}={:.3}",
                            updated_weights.len(),
                            updated_weights.keys().next().unwrap_or(&"none".to_string()),
                            updated_weights.values().next().copied().unwrap_or(0.0)
                        );

                        if let Some(new_config) = evolved_config {
                            println!(
                                "[SelfEvolution] Meta-control adapted: max_risk_per_trade_pct={:.4}, max_leverage={}",
                                new_config.max_risk_per_trade_pct,
                                new_config.absolute_max_leverage
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "[SelfEvolution] Outcome processing failed for {}: {}",
                            sym_clone, e
                        );
                    }
                }
            });
        }
    }

    async fn check_emergency_meta_today(&self) {
        let portfolio = self.state.portfolio.read().await;
        let losing_today = portfolio.losing_trades_today;
        drop(portfolio);

        if losing_today >= 3 {
            println!(
                "[ExecutionCoordinator] 🚨 {} losing trades today — triggering emergency MetaControl review",
                losing_today
            );
            let meta = crate::meta_control::MetaControlAgent::new(self.state.clone());
            if let Err(e) = meta.tune_skill_weights(1).await {
                eprintln!("[ExecutionCoordinator] Emergency meta failed: {}", e);
            }

            let _ = self
                .state
                .push_cot(
                    "MetaControl",
                    "Emergency review triggered",
                    "RULE_TIGHTEN",
                    &format!("{} losing trades today — reviewing rules", losing_today),
                    0.95,
                    0,
                    None,
                    None,
                )
                .await;
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod compliance_tests {
    // Compliance integration tests require running compliance gateway
    // These are tested at the crate level in tredo-compliance
}
