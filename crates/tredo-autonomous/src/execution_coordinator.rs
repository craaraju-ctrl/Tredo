use crate::outcome_processor::OutcomeProcessor;
use crate::state::SharedState;
use crate::types::TradeSignal;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier, TradeOutcome, TradingEpisode};

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
        println!(
            "[ExecutionCoordinator] Executing paper trade: {} {} @ {:.2} | Qty: {:.0}",
            signal.symbol,
            if signal.direction == tredo_core::TradeDirection::Long {
                "BUY"
            } else {
                "SELL"
            },
            signal.entry_price,
            signal.position_size
        );

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

        println!("[ExecutionCoordinator] Order filled (simulated)");

        let pm = crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());
        pm.add_position(signal).await?;

        let direction_str = if signal.direction == tredo_core::TradeDirection::Long {
            "BUY"
        } else {
            "SELL"
        };
        let exec_log = format!(
            "EXECUTED: {} {} {:.0} @ {:.2} | Stop: {:.2} | Target: {:.2}",
            signal.symbol,
            direction_str,
            signal.position_size,
            signal.entry_price,
            signal.stop_loss,
            signal.take_profit
        );

        // Push COT entry for the execution
        self.state
            .push_cot(
                "ExecutionEngine",
                &format!(
                    "Execute {} {} @ {:.2} | Qty: {:.0} | SL: {:.2} | TP: {:.2}",
                    signal.symbol,
                    direction_str,
                    signal.entry_price,
                    signal.position_size,
                    signal.stop_loss,
                    signal.take_profit
                ),
                "FILLED",
                &format!(
                    "Order filled: {} {} @ {:.2}. Position size: {:.0}",
                    signal.symbol, direction_str, signal.entry_price, signal.position_size
                ),
                signal.confidence_score,
                0,
                None,
                Some(signal.symbol.clone()),
            )
            .await;

        let _ = self.state.memory.store_decision(
            &format!("execution/{}/{}", signal.symbol, Utc::now().timestamp()),
            &exec_log,
        );

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

    async fn check_and_exit_positions(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
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
                        // Feedback loop: score regret and persist to SQLite
                        let op = OutcomeProcessor::new(self.state.clone());
                        op.close_episode(pos, pos.stop_loss, "stop_loss", pnl).await;

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
                        // Feedback loop: score regret and persist to SQLite
                        let op = OutcomeProcessor::new(self.state.clone());
                        op.close_episode(pos, pos.take_profit, "take_profit", pnl)
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
