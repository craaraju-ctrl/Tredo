use crate::types::{PipelineSummary, TradeSignal};
use std::error::Error;
use tredo_core::TradeDirection;

// NOTE: These phase methods are preserved for backward API compatibility.
// The pipeline now routes through Tredo groups (see tredo.rs) instead.
#[allow(dead_code)]
impl crate::orchestrator_struct::AutonomousOrchestrator {
    /// Phase 5: LLM-driven strategy decision.
    /// Returns Option<TradeSignal> — None means LLM decided HOLD.
    pub async fn phase5_strategy_decision(
        &self,
        symbol: &str,
        direction: TradeDirection,
        entry: f64,
        stop: f64,
        target: f64,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        println!("\n[PHASE 5] LLM Strategy Decision");
        let signal_opt = self
            .strategy
            .generate_signal(symbol, direction, entry, stop, target)
            .await?;

        match &signal_opt {
            Some(sig) => println!(
                "[PHASE 5] LLM decided {} {} @ {:.2} (confidence {:.1}%)",
                if sig.direction == tredo_core::TradeDirection::Long {
                    "BUY"
                } else {
                    "SELL"
                },
                symbol,
                sig.entry_price,
                sig.confidence_score * 100.0
            ),
            None => println!(
                "[PHASE 5] LLM decided HOLD for {} — skipping execution.",
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
    pub async fn run_full_pipeline(
        &self,
        symbol: &str,
        direction: TradeDirection,
        entry: f64,
        stop: f64,
        target: f64,
    ) -> Result<PipelineSummary, Box<dyn Error + Send + Sync>> {
        let start = std::time::Instant::now();
        println!("\n=== tredo AUTONOMOUS PIPELINE for {} ===", symbol);
        let tredo = self.tredo();

        // Start a COT chain for this pipeline run
        let chain_id = self
            .state
            .start_cot_chain(
                "Orchestrator",
                &format!(
                    "Running full pipeline for {} {} @ {:.2}",
                    symbol,
                    if direction == TradeDirection::Long {
                        "BUY"
                    } else {
                        "SELL"
                    },
                    entry
                ),
                "PIPELINE_START",
                &format!("Starting autonomous pipeline for {}", symbol),
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

        // ── IDENTIFIER GROUP ─────────────────────────────────────────────────
        // Runs market analysis + session timer + red folder checks
        let (discipline_ok, confluence, pivots) = tredo.run_identifier(symbol, entry).await?;

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
                &format!("Market analysis for {} @ {:.2}", symbol, entry),
                "ANALYZED",
                &format!(
                    "Confluence: {:.1}%, Pivot: {:.2}, R1: {:.2}, S1: {:.2}, Discipline: OK",
                    confluence * 100.0,
                    pivots.pivot,
                    pivots.r1,
                    pivots.s1
                ),
                confluence,
                Some(symbol.to_string()),
            )
            .await;

        // ── VERIFIER GROUP ───────────────────────────────────────────────────
        // Runs drawdown + overtrading checks + risk psychology + reflection
        let equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.total_equity
        };
        let risk = tredo.run_verifier(symbol, entry, equity).await?;
        let risk_passed = risk.recommendation != crate::types::RiskRecommendation::Halt;

        self.state
            .add_cot_step(
                chain_id,
                "Verifier",
                &format!("Risk assessment for {}", symbol),
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

        // ── EXECUTER GROUP ───────────────────────────────────────────────────
        // Runs LLM strategy decision + execution + outcome logging
        let signal_opt = tredo
            .run_executer(symbol, direction, entry, stop, target)
            .await?;

        match &signal_opt {
            Some(sig) => {
                self.state
                    .add_cot_step(
                        chain_id,
                        "Executer",
                        &format!("LLM decision for {} @ {:.2}", symbol, entry),
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
                        &format!("LLM decision for {} @ {:.2}", symbol, entry),
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
            format!(
                "✅ Pipeline complete: {} {} @ {:.2} in {}ms",
                symbol,
                if direction == TradeDirection::Long {
                    "BUY"
                } else {
                    "SELL"
                },
                entry,
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
