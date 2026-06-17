use crate::types::{PipelineSummary, TradeSignal};
use std::error::Error;
use tredo_core::{Agent, TradeDirection};

// NOTE: These phase methods are preserved for backward API compatibility.
// The pipeline now routes through Tredo groups (see tredo.rs) instead.
#[allow(dead_code)]
impl crate::orchestrator_struct::AutonomousOrchestrator {
    /// Phase 5: **Agentic** strategy decision (no pre-supplied price points).
    /// The agent autonomously identifies entry, SL, TP, direction using its full stack (indicators, debate, memory, rules).
    /// This is what makes it agentic AI rather than a scripted bot.
    pub async fn phase5_strategy_decision(
        &self,
        symbol: &str,
        current_price: f64,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 5] Agentic (autonomous) Strategy Decision");
        let signal_opt = self.strategy.generate_signal(symbol, current_price).await?;

        match &signal_opt {
            Some(sig) => println!(
                "[PHASE 5] AGENT decided {} {} @ entry={:.2} SL={:.2} TP={:.2} (confidence {:.1}%)",
                if sig.direction == tredo_core::TradeDirection::Long {
                    "BUY"
                } else {
                    "SELL"
                },
                symbol,
                sig.entry_price,
                sig.stop_loss,
                sig.take_profit,
                sig.confidence_score * 100.0
            ),
            None => println!(
                "[PHASE 5] AGENT decided HOLD for {} — skipping execution.",
                symbol
            ),
        }

        Ok(signal_opt)
    }

    /// Phase 6: Execute the paper trade and update the portfolio.
    pub async fn phase6_portfolio_and_execution(
        &self,
        signal: &TradeSignal,
    ) -> Result<bool, Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 6] Portfolio & Execution");
        let result = self.execution.execute_paper_trade(signal).await?;
        println!("[PHASE 6] {}", result);
        Ok(true)
    }

    /// Full autonomous pipeline with proper 5-layer hierarchy:
    ///
    /// ```text
    /// Phase 0: Position check
    /// Layer 1: HardRulesGate (ALL hard rules — NEVER overridden)
    /// Layer 2: Identifier (data gathering, COT entries, confluence/pivots)
    ///         Verifier (risk analysis, position sizing — advisory)
    /// Layer 3: DebateLayer (advisory — 6 agents, no veto power)
    /// Layer 4: Judge (final adjudication — only evaluates debate quality)
    /// Layer 5: Execute
    /// ```
    ///
    /// Key principle: HardRulesGate runs FIRST. If it blocks, no agents run.
    /// Identifier/Verifier gather data but never block — the gate already handled hard rules.
    /// Debate agents are ADVISORY only — only the Judge has decision-making power.
    pub async fn run_full_pipeline(
        &self,
        symbol: &str,
    ) -> Result<PipelineSummary, Box<dyn Error + Send + Sync>> {
        let start = std::time::Instant::now();
        println!(
            "\n=== tredo AUTONOMOUS (AGENTIC) PIPELINE for {} ===",
            symbol
        );

        // Agent perceives the current market price from its state
        let observed_price = {
            let history = self.state.ohlcv_history.read().await;
            history
                .get(symbol)
                .and_then(|bars| bars.last().map(|b| b.close))
                .unwrap_or(0.0)
        };

        if observed_price <= 0.0 {
            return Ok(PipelineSummary {
                executed: false,
                phase_results: vec![],
                total_duration_ms: start.elapsed().as_millis() as u64,
                final_signal: None,
                reason: format!("No observable market data for {}", symbol),
            });
        }

        // Start a COT chain for this pipeline run
        let chain_id = self
            .state
            .start_cot_chain(
                "Orchestrator",
                &format!(
                    "Running full agentic pipeline for {} (observed market price {:.2})",
                    symbol, observed_price
                ),
                "PIPELINE_START",
                &format!("Starting fully autonomous agentic pipeline for {}", symbol),
                1.0,
            )
            .await;

        // ── Phase 0: Check if there is already an open position on this symbol ──
        {
            let portfolio = self.state.portfolio.read().await;
            if portfolio
                .open_positions
                .iter()
                .any(|pos| pos.symbol == symbol)
            {
                self.state
                    .add_cot_step(
                        chain_id, "Phase0", "Checking existing positions", "SKIP",
                        &format!("Already have an open position for {}", symbol),
                        1.0, Some(symbol.to_string()),
                    )
                    .await;
                return Ok(PipelineSummary {
                    executed: false,
                    phase_results: vec![],
                    total_duration_ms: start.elapsed().as_millis() as u64,
                    final_signal: None,
                    reason: format!("Already have an open position for {}", symbol),
                });
            }
        }
        self.state
            .add_cot_step(
                chain_id, "Phase0", "Checking existing positions", "PASS",
                "No existing position — proceeding",
                1.0, Some(symbol.to_string()),
            )
            .await;

        // ═══ LAYER 1: HARD RULES GATE (runs FIRST — no agents run if this fails) ═══
        // Single top-level enforcement of ALL hard rules with priority-based conflict resolution.
        // Critical/High rules block trading. Medium rules block if no Higher rule overrides.
        // Low-priority failures are WARNINGS only — they log but don't block.
        let hard_rules = crate::hard_rules_gate::HardRulesGate::new(self.state.clone());
        let gate_result = hard_rules.evaluate(symbol).await;

        if !gate_result.passed {
            self.state
                .add_cot_step(
                    chain_id, "HardRulesGate",
                    &format!("Hard rules check for {} ({} rules checked)", symbol, gate_result.total_rules_checked),
                    "BLOCKED",
                    &format!("Highest priority: {:?}. Failed rules: {}",
                        gate_result.highest_failed_priority.unwrap_or(crate::types::RulePriority::Low),
                        gate_result.failed_rules.iter().map(|r| r.rule_name.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                    0.0, Some(symbol.to_string()),
                )
                .await;
            return Ok(PipelineSummary {
                executed: false,
                phase_results: vec![],
                total_duration_ms: start.elapsed().as_millis() as u64,
                final_signal: None,
                reason: format!("Hard Rules Gate blocked: {} (priority {:?})",
                    gate_result.failed_rules.first().map(|r| r.reason.as_str()).unwrap_or("unknown"),
                    gate_result.highest_failed_priority
                ),
            });
        }

        self.state
            .add_cot_step(
                chain_id, "HardRulesGate",
                &format!("Hard rules check for {} ({} rules checked)", symbol, gate_result.total_rules_checked),
                "PASSED",
                &format!("All {} hard rules passed", gate_result.total_rules_checked),
                1.0, Some(symbol.to_string()),
            )
            .await;

        // ═══ LAYER 2: IDENTIFIER (data gathering — advisory only, never blocks) ═══════
        // Runs all 7 sub-agents to gather market intelligence.
        // Session/red_folder checks are now informational COT entries only —
        // the HardRulesGate already enforced these as Critical rules.
        let tredo = self.tredo();
        let (discipline_ok, confluence, pivots) = tredo
            .run_identifier(symbol, observed_price, chain_id)
            .await?;

        // Log discipline status as informational (gate already handled blocking)
        if !discipline_ok {
            self.state
                .add_cot_step(
                    chain_id, "Identifier", &format!("Discipline checks for {} (informational)", symbol),
                    "INFO",
                    "Session/red_folder check flagged — already enforced by HardRulesGate",
                    0.8, Some(symbol.to_string()),
                )
                .await;
        }

        self.state
            .add_cot_step(
                chain_id, "Identifier",
                &format!("Market analysis for {} @ {:.2}", symbol, observed_price),
                "ANALYZED",
                &format!(
                    "Confluence: {:.1}%, Pivot: {:.2}, R1: {:.2}, S1: {:.2}",
                    confluence * 100.0, pivots.pivot, pivots.r1, pivots.s1
                ),
                confluence, Some(symbol.to_string()),
            )
            .await;

        // ═══ LAYER 2: VERIFIER (risk analysis — advisory only, never blocks) ═════════
        // Runs risk psychology, risk calculator, reflector.
        // Drawdown/overtrading checks are informational — the HardRulesGate enforced these.
        let equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.total_equity
        };
        let risk = tredo
            .run_verifier(symbol, observed_price, equity, chain_id)
            .await?;

        // Log risk status as informational (gate already handled blocking)
        self.state
            .add_cot_step(
                chain_id, "Verifier",
                &format!("Risk assessment for {} (observed price {:.2})", symbol, observed_price),
                "ANALYZED",
                &format!(
                    "Heat: {:.1}%, DD: {:.1}%, Recommendation: {:?}",
                    risk.portfolio_heat * 100.0, risk.daily_drawdown_pct * 100.0, risk.recommendation
                ),
                (1.0 - risk.portfolio_heat).max(0.0), Some(symbol.to_string()),
            )
            .await;

        // ═══ PRECISION GATE: Regime Consistency Check ═══════════════════════════
        // Lightweight spot-check on first trade of day: verify recent price action
        // is consistent with the declared regime.
        {
            let portfolio = self.state.portfolio.read().await;
            let total_trades = portfolio.total_trades_today;
            drop(portfolio);

            if total_trades == 0 {
                let bars = {
                    let hist = self.state.ohlcv_history.read().await;
                    hist.get(symbol).cloned().unwrap_or_default()
                };

                if bars.len() >= 100 {
                    let recent_closes: Vec<f64> = bars.iter().rev().take(20).map(|b| b.close).collect();
                    let recent_trend = if recent_closes.len() >= 2 {
                        (recent_closes[0] - recent_closes[recent_closes.len() - 1]) / recent_closes[recent_closes.len() - 1]
                    } else {
                        0.0
                    };
                    let regime = *self.state.market_regime.read().await;

                    let regime_consistent = match &regime {
                        Some(crate::types::MarketRegime::TrendingBull) => recent_trend > -0.005,
                        Some(crate::types::MarketRegime::TrendingBear) => recent_trend < 0.005,
                        _ => true,
                    };

                    if !regime_consistent {
                        self.state
                            .add_cot_step(
                                chain_id, "WFA_Gate",
                                &format!("Regime consistency check for {}", symbol),
                                "REJECT",
                                &format!("Regime {:?} inconsistent with recent trend ({:.3}%). WFA gate blocking.", regime, recent_trend * 100.0),
                                0.1, Some(symbol.to_string()),
                            )
                            .await;
                        return Ok(PipelineSummary {
                            executed: false,
                            phase_results: vec![],
                            total_duration_ms: start.elapsed().as_millis() as u64,
                            final_signal: None,
                            reason: format!("WFA gate: Regime {:?} inconsistent with recent price action", regime),
                        });
                    }

                    self.state
                        .add_cot_step(
                            chain_id, "WFA_Gate",
                            &format!("Regime consistency check for {}", symbol),
                            "PASS",
                            &format!("Regime {:?} consistent with recent trend ({:.3}%)", regime, recent_trend * 100.0),
                            0.9, Some(symbol.to_string()),
                        )
                        .await;
                }
            }
        }

        // ═══ LAYER 3: DEBATE LAYER (Advisory Only) ════════════════════════════
        // Multi-round adversarial decision: Bull Team vs Bear Team → Synthesizer → Judge
        // NOTE: Debate agents are ADVISORY only. They provide evidence + confidence.
        // Only the Judge (Layer 4) has decision-making power.
        let debate_layer = crate::debate_layer::DebateLayer::new(self.state.clone());
        let (verdict, signal_opt) = debate_layer.run_debate(symbol, observed_price).await;

        self.state
            .add_cot_step(
                chain_id, "DebateLayer",
                &format!("Adversarial debate for {} ({} rounds)", symbol, verdict.rounds_played),
                &verdict.action,
                &format!(
                    "Confidence: {:.1}%, Judge veto: {}, Rounds: {}",
                    verdict.confidence * 100.0, verdict.judge_veto, verdict.rounds_played
                ),
                verdict.confidence, Some(symbol.to_string()),
            )
            .await;

        // ═══ LAYER 5: EXECUTE TRADE (if debate layer approved) ══════════════════
        let executed = if let Some(ref sig) = signal_opt {
            match self.execution.execute_paper_trade(sig).await {
                Ok(result) => {
                    println!("[Pipeline] ✅ Trade executed: {}", result);

                    self.state
                        .add_cot_step(
                            chain_id, "ExecutionCoordinator",
                            &format!("Executing {} paper trade", symbol),
                            "EXECUTED",
                            &result,
                            sig.confidence_score, Some(symbol.to_string()),
                        )
                        .await;

                    let _ = self.outcome_logger.run(None).await;
                    self.state
                        .add_cot_step(
                            chain_id, "OutcomeLogger",
                            "Logging trade outcome", "LOGGED",
                            &format!("Trade logged for {} {:?}", symbol, sig.direction),
                            0.8, Some(symbol.to_string()),
                        )
                        .await;

                    true
                }
                Err(e) => {
                    println!("[Pipeline] ❌ Trade execution failed: {}", e);
                    false
                }
            }
        } else {
            self.state
                .add_cot_step(
                    chain_id, "Executer",
                    &format!("AGENTIC HOLD for {}", symbol),
                    "HOLD", "Debate layer decided HOLD — no trade placed",
                    0.0, Some(symbol.to_string()),
                )
                .await;
            false
        };

        // ── Pipeline completion ──────────────────────────────────────────────
        let total_ms = start.elapsed().as_millis() as u64;
        let exec_reason = if executed {
            let sig = signal_opt.as_ref().unwrap();
            format!(
                "✅ Pipeline complete: {} {} @ entry {:.2} (SL {:.2} TP {:.2}) in {}ms",
                symbol,
                if sig.direction == TradeDirection::Long { "BUY" } else { "SELL" },
                sig.entry_price, sig.stop_loss, sig.take_profit, total_ms
            )
        } else {
            format!(
                "Pipeline complete: HOLD for {} in {}ms",
                symbol, total_ms
            )
        };

        self.state
            .add_cot_step(
                chain_id, "Decision", "Pipeline final decision",
                if executed { "TRADE_EXECUTED" } else { "HOLD" },
                &exec_reason,
                if executed { 0.9 } else { 0.5 },
                Some(symbol.to_string()),
            )
            .await;

        Ok(PipelineSummary {
            executed,
            phase_results: vec![],
            total_duration_ms: total_ms,
            final_signal: signal_opt,
            reason: exec_reason,
        })
    }
}
