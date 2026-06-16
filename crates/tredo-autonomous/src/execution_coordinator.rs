use crate::outcome_processor::OutcomeProcessor;
use crate::state::SharedState;
use crate::types::TradeSignal;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use tredo_core::paper_engine::{OrderRequest, OrderType, TradingMode};
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
    use crate::weight_tuner::AttributionEngine;
    use crate::meta_control::EvolvedMetaControl;

    let weight_tuner = AttributionEngine::new(0.10);
    let meta_control = EvolvedMetaControl::new(db_for_meta, 0.05, 1);
    let processor = Arc::new(OutcomeProcessor::new(episode_store, weight_tuner, meta_control));

    let mut guard = OUTCOME_PROCESSOR.lock().await;
    *guard = Some(processor.clone());
    println!("[OutcomeProcessor] 🧬 Global OutcomeProcessor initialized — self-evolution loop active");
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

    pub async fn execute_paper_trade(
        &self,
        signal: &TradeSignal,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        if signal.position_size <= 0.0 {
            self.state
                .push_cot(
                    "ExecutionEngine",
                    &format!(
                        "Execute {} {} @ {:.2}",
                        signal.symbol,
                        if signal.direction == tredo_core::TradeDirection::Long {
                            "BUY"
                        } else {
                            "SELL"
                        },
                        signal.entry_price
                    ),
                    "REJECTED",
                    "Invalid position size — cannot execute",
                    0.0,
                    0,
                    None,
                    Some(signal.symbol.clone()),
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
            if signal.direction == tredo_core::TradeDirection::Long { "BUY" } else { "SELL" },
            effective_entry, signal.entry_price,
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
        pm.add_position(&adjusted_signal).await?;

        let direction_str = if adjusted_signal.direction == tredo_core::TradeDirection::Long {
            "BUY"
        } else {
            "SELL"
        };
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

        self.state
            .push_cot(
                "ExecutionEngine",
                &format!(
                    "Execute {} {} @ {:.2}",
                    adjusted_signal.symbol, direction_str, adjusted_signal.entry_price
                ),
                "FILLED",
                &exec_log,
                adjusted_signal.confidence_score,
                0,
                None,
                Some(adjusted_signal.symbol.clone()),
            )
            .await;

        let _ = self.state.memory.store_decision(
            &format!("execution/{}/{}", signal.symbol, Utc::now().timestamp()),
            &exec_log,
        );

        if let Some(processor) = get_outcome_processor() {
            let snapshot = build_pre_trade_snapshot(&self.state, &adjusted_signal).await;
            processor.register_pending_trade(snapshot);
        }

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
                                    let mut v = vm.lock().await;
                                    let _ = v
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

    /// Execute a trade through the live broker (Zerodha Kite API).
    async fn execute_live_trade(
        &self,
        signal: &TradeSignal,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
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
            signal.symbol, direction_str, order_req.qty, signal.entry_price, signal.stop_loss, signal.take_profit
        );

        match broker.place_order(order_req.clone(), signal.entry_price).await {
            Ok(order_id) => {
                let exec_log = format!(
                    "LIVE EXECUTED: {} {} qty={} @ {:.2} | Order: {} | SL: {:.2} | TP: {:.2}",
                    signal.symbol, direction_str, order_req.qty, signal.entry_price, order_id,
                    signal.stop_loss, signal.take_profit
                );

                self.state
                    .push_cot(
                        "ExecutionEngine",
                        &format!("LIVE {} {} @ {:.2}", signal.symbol, direction_str, signal.entry_price),
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

                Ok(format!("Live trade executed: {}", exec_log))
            }
            Err(e) => {
                let err_msg = format!("LIVE TRADE FAILED: {} {} — {}", signal.symbol, direction_str, e);
                eprintln!("[ExecutionCoordinator] {}", err_msg);

                self.state
                    .push_cot(
                        "ExecutionEngine",
                        &format!("LIVE {} {} @ {:.2}", signal.symbol, direction_str, signal.entry_price),
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
        // In live mode, positions are managed by the broker (SL/TP placed on exchange).
        // We skip paper SL/TP checks when live and let the exchange handle exits.
        let mode = self.state.broker_registry.current_mode().await;
        if mode == TradingMode::Live {
            // Still sync position P&L for display but don't auto-exit
            return Ok(Vec::new());
        }

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
                        let episode_id = format!("ep-{}-{}", pos.symbol, pos.entry_time.timestamp());
                        self.spawn_outcome_processing(episode_id, exit_price, pos.symbol.clone()).await;

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

                        // === NEW: Trigger OutcomeProcessor for self-evolution ===
                        // Use pos.entry_time.timestamp() to match the episode_id registered in execute_paper_trade
                        let exit_price = pos.take_profit;
                        let episode_id = format!("ep-{}-{}", pos.symbol, pos.entry_time.timestamp());
                        self.spawn_outcome_processing(episode_id, exit_price, pos.symbol.clone()).await;

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
                        // Push adaptation event to COT for observability in TUI (self-evolution visible)
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
        votes.iter()
            .map(|v| (v.skill_name.clone(), v.score))
            .collect()
    };

    let direction_str = match signal.direction {
        tredo_core::TradeDirection::Long => "BUY",
        tredo_core::TradeDirection::Short => "SELL",
    };

    crate::outcome_processor::PreTradeSnapshot {
        episode_id: format!("ep-{}-{}", signal.symbol, Utc::now().timestamp()),
        symbol: signal.symbol.clone(),
        direction: direction_str.to_string(),
        entry_price: signal.entry_price,
        rule_version: 1,
        active_weights,
        skill_predictions,
    }
}

// =====================================================================
// Helper: spawn OutcomeProcessor as a background task (reduces code duplication)
// =====================================================================
impl ExecutionCoordinatorAgent {
    async fn spawn_outcome_processing(&self, episode_id: String, exit_price: f64, symbol: String) {
        if let Some(processor) = get_outcome_processor() {
            let current_regime = {
                let regime = self.state.market_regime.read().await;
                match *regime {
                    Some(crate::types::MarketRegime::TrendingBull) =>
                        crate::regime_classifier::MarketRegime::TrendingBull,
                    Some(crate::types::MarketRegime::TrendingBear) =>
                        crate::regime_classifier::MarketRegime::TrendingBear,
                    Some(crate::types::MarketRegime::Ranging) =>
                        crate::regime_classifier::MarketRegime::Ranging,
                    Some(crate::types::MarketRegime::Volatile) =>
                        crate::regime_classifier::MarketRegime::Volatile,
                    _ => crate::regime_classifier::MarketRegime::Ranging,
                }
            };

            let current_config = crate::risk_guardian::RiskGuardianConfig::default_fallback();

            let proc_clone = processor.clone();
            let eid_clone = episode_id.clone();
            let sym_clone = symbol.clone();
            tokio::spawn(async move {
                match proc_clone.process_trade_close(
                    &eid_clone,
                    exit_price,
                    current_regime,
                    10,
                    &current_config,
                ).await {
                    Ok((_weights, evolved_config)) => {
                        if let Some(new_config) = evolved_config {
                            println!(
                                "[SelfEvolution] Meta-control adapted: max_risk_per_trade_pct={:.4}, max_leverage={}",
                                new_config.max_risk_per_trade_pct,
                                new_config.absolute_max_leverage
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!("[SelfEvolution] Outcome processing failed for {}: {}", sym_clone, e);
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

            let _ = self.state.push_cot(
                "MetaControl",
                "Emergency review triggered",
                "RULE_TIGHTEN",
                &format!("{} losing trades today — reviewing rules", losing_today),
                0.95,
                0,
                None,
                None,
            ).await;
        }
    }
}
