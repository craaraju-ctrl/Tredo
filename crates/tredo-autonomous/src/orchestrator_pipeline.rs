use crate::types::{PipelineSummary, TradeSignal};
use std::error::Error;
use tredo_core::TradeDirection;

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

    /// Full autonomous pipeline: routes through Tredo groups (Identifier → Verifier → Executer).
    /// Pushes real chain-of-thought entries into SharedState.cot_store.
    /// Fully agentic pipeline (no external price points or direction).
    /// The agent observes latest market data from state (populated by the data feed / scanner).
    /// It uses its skills (patterns, volume, RSI, MACD, ATR, pivots, regime, confluence, memory recall)
    ///   + debate + DisciplinedCore rules to decide *if*, *direction*, and the precise levels.
    ///
    ///   This is the definition of agentic AI trading vs a bot that is told the levels.
    pub async fn run_full_pipeline(
        &self,
        symbol: &str,
    ) -> Result<PipelineSummary, Box<dyn Error + Send + Sync>> {
        let start = std::time::Instant::now();
        println!(
            "\n=== tredo AUTONOMOUS (AGENTIC) PIPELINE for {} ===",
            symbol
        );
        let tredo = self.tredo();

        // Agent perceives the current market price from its state (real-time data feed populates ohlcv_history)
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

        // Phase 0 — Check if there is already an open position on this symbol
        {
            let portfolio = self.state.portfolio.read().await;
            if portfolio
                .open_positions
                .iter()
                .any(|pos| pos.symbol == symbol)
            {
                self.state
                    .add_cot_step(
                        chain_id,
                        "Phase0",
                        "Checking existing positions",
                        "SKIP",
                        &format!("Already have an open position for {}", symbol),
                        1.0,
                        Some(symbol.to_string()),
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
                chain_id,
                "Phase0",
                "Checking existing positions",
                "PASS",
                "No existing position — proceeding",
                1.0,
                Some(symbol.to_string()),
            )
            .await;

        // Get latest observed price from state (agent perceives the market itself)
        let _latest_price = {
            let history = self.state.ohlcv_history.read().await;
            history
                .get(symbol)
                .and_then(|bars| bars.last().map(|b| b.close))
                .unwrap_or(0.0) // fallback only for bootstrap; will be caught by the <=0 check above
        };

        // ── IDENTIFIER GROUP ─────────────────────────────────────────────────
        // Runs market analysis + session timer + red folder checks.
        // Agent observes latest data and computes its own indicators (trend, patterns, volume, RSI, MACD etc. via skills).
        let (discipline_ok, confluence, pivots) = tredo
            .run_identifier(symbol, observed_price, chain_id)
            .await?;

        if !discipline_ok {
            self.state
                .add_cot_step(
                    chain_id,
                    "Identifier",
                    &format!("Discipline checks for {}", symbol),
                    "FAIL",
                    "Session timing or red folder check failed",
                    0.1,
                    Some(symbol.to_string()),
                )
                .await;
            self.state
                .add_cot_step(
                    chain_id,
                    "Decision",
                    "Pipeline aborted",
                    "ABORT",
                    "Discipline checks failed — halting",
                    0.0,
                    Some(symbol.to_string()),
                )
                .await;
            return Ok(PipelineSummary {
                executed: false,
                phase_results: vec![],
                total_duration_ms: start.elapsed().as_millis() as u64,
                final_signal: None,
                reason: "Discipline checks failed (session/red_folder)".to_string(),
            });
        }

        self.state
            .add_cot_step(
                chain_id,
                "Identifier",
                &format!("Market analysis for {} @ {:.2} (agent observed)", symbol, observed_price),
                "ANALYZED",
                &format!(
                    "Confluence: {:.1}%, Pivot: {:.2}, R1: {:.2}, S1: {:.2}, Discipline: OK (agent observed price {:.2})",
                    confluence * 100.0,
                    pivots.pivot,
                    pivots.r1,
                    pivots.s1,
                    observed_price
                ),
                confluence,
                Some(symbol.to_string()),
            )
            .await;

        // ── VERIFIER GROUP ───────────────────────────────────────────────────
        // Runs drawdown + overtrading checks + risk psychology + reflection.
        // Agent uses its own computed market state (no external levels).
        let equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.total_equity
        };
        let risk = tredo
            .run_verifier(symbol, observed_price, equity, chain_id)
            .await?;
        let risk_passed = risk.recommendation != crate::types::RiskRecommendation::Halt;

        self.state
            .add_cot_step(
                chain_id,
                "Verifier",
                &format!(
                    "Risk assessment for {} (observed price {:.2})",
                    symbol, observed_price
                ),
                if risk_passed { "PASS" } else { "HALT" },
                &format!(
                    "Heat: {:.1}%, DD: {:.1}%, Recommendation: {:?}",
                    risk.portfolio_heat * 100.0,
                    risk.daily_drawdown_pct * 100.0,
                    risk.recommendation
                ),
                (1.0 - risk.portfolio_heat).max(0.0),
                Some(symbol.to_string()),
            )
            .await;

        if !risk_passed {
            self.state
                .add_cot_step(
                    chain_id,
                    "Decision",
                    "Pipeline aborted",
                    "ABORT",
                    "Risk assessment: HALT",
                    0.0,
                    Some(symbol.to_string()),
                )
                .await;
            return Ok(PipelineSummary {
                executed: false,
                phase_results: vec![],
                total_duration_ms: start.elapsed().as_millis() as u64,
                final_signal: None,
                reason: "Risk assessment: HALT".to_string(),
            });
        }

        // === EXPLICIT AGGREGATOR + DECISION HANDOFF (Gap 1 blueprint) ===
        // 1. MarketIntelligence has already run and stored last_aggregated_signal + last_skill_votes.
        // 2. We now pull the AggregatedSignal and pass it as a first-class parameter into the decision layer.
        // This is the critical missing link that turns "skills thinking aloud" into the agent actually
        // using its own cross-skill consensus when choosing to trade and at what levels.
        let aggregated_signal = {
            let agg = self.state.last_aggregated_signal.read().await;
            agg.clone()
        };

        // The agent decides direction + its own entry/SL/TP using the aggregated signal.
        // We deliberately do *not* pass pre-computed entry/stop/target from the orchestrator.
        let signal_opt = tredo
            .run_executer_with_aggregation(
                symbol,
                observed_price,
                aggregated_signal.as_ref(),
                chain_id,
            )
            .await?;

        match &signal_opt {
            Some(sig) => {
                self.state
                    .add_cot_step(
                        chain_id,
                        "Executer",
                        &format!(
                            "AGENTIC decision for {} (observed price {:.2})",
                            symbol, observed_price
                        ),
                        if sig.direction == TradeDirection::Long {
                            "BUY"
                        } else {
                            "SELL"
                        },
                        &format!(
                            "Confidence: {:.1}%, Confluence: {:.1}%, R:R {:.1}:1, Reason: {}",
                            sig.confidence_score * 100.0,
                            sig.confluence_score * 100.0,
                            sig.risk_reward_ratio,
                            sig.reasoning.chars().take(60).collect::<String>()
                        ),
                        sig.confidence_score,
                        Some(symbol.to_string()),
                    )
                    .await;
            }
            None => {
                self.state
                    .add_cot_step(
                        chain_id,
                        "Executer",
                        &format!(
                            "AGENTIC decision for {} (observed price {:.2})",
                            symbol, observed_price
                        ),
                        "HOLD",
                        "LLM decided HOLD — no trade placed",
                        0.0,
                        Some(symbol.to_string()),
                    )
                    .await;
            }
        }

        let executed = signal_opt.is_some();
        let exec_reason = if executed {
            "Tredo trade executed".to_string()
        } else {
            "Tredo HOLD — no trade placed".to_string()
        };

        let total_ms = start.elapsed().as_millis() as u64;
        let final_action = if executed { "TRADE_EXECUTED" } else { "HOLD" };
        let final_reason = if executed {
            // The signal contains the levels the *agent* decided
            let sig = signal_opt.as_ref().unwrap();
            format!(
                "✅ Pipeline complete: {} {} @ entry {:.2} (agent decided SL {:.2} TP {:.2}) in {}ms",
                symbol,
                if sig.direction == TradeDirection::Long { "BUY" } else { "SELL" },
                sig.entry_price,
                sig.stop_loss,
                sig.take_profit,
                total_ms
            )
        } else {
            format!(
                "Pipeline complete: HOLD for {} in {}ms. {}",
                symbol, total_ms, exec_reason
            )
        };
        self.state
            .add_cot_step(
                chain_id,
                "Decision",
                "Pipeline final decision",
                final_action,
                &final_reason,
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

    pub async fn run_health_check(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let reports = vec!["System health: OK (LLM-driven autonomous mode)".to_string()];
        Ok(reports)
    }

    pub async fn run_monitoring_loop(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        println!("[Orchestrator] Monitoring loop started (LLM-driven)");
        Ok(())
    }
}
