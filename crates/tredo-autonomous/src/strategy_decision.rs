// ═══════════════════════════════════════════════════════════════════════════════
// Strategy Decision Agent — UPGRADED ARCHITECTURE
//
// NEW DECISION FLOW (LLM-optional):
//   1. Compute indicators (RSI, MACD, ATR, regime, pivots)
//   2. Run DETERMINISTIC STRATEGIES FIRST (no LLM) → primary signal
//   3. Run debate layer for richer multi-agent reasoning
//   4. Run SuperIntelligence cross-validation + conviction stacking
//   5. LLM runs as CROSS-CHECK OPINION ONLY (demoted from primary)
//   6. Apply Behavioral Psychology sizing adjustment
//   7. Produce final TradeSignal
//
// Key principle: The system works WITHOUT LLM or Kronos forecast.
// LLM is just an opinion layer to cross-check the deterministic + SI result.
// ═══════════════════════════════════════════════════════════════════════════════

use crate::helpers::get_indian_session_info;
use crate::state::SharedState;
use crate::types::{MarketRegime, TradeSignal};
use chrono::Utc;
use std::error::Error;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, validate_trade_setup, AgentInput,
    LlmTradeDecision, MarketContext,
};

pub struct StrategyDecisionAgent {
    pub state: SharedState,
}

impl StrategyDecisionAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Generate a trade signal using the upgraded 5-step decision flow.
    /// LLM is optional — system works fully deterministically.
    pub async fn generate_signal(
        &self,
        symbol: &str,
        current_price: f64,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        let aggregated = {
            let a = self.state.last_aggregated_signal.read().await;
            a.clone()
        };
        self.generate_signal_with_aggregation(symbol, current_price, aggregated.as_ref())
            .await
    }

    /// Full decision with AggregatedSignal (preferred path).
    /// Deterministic strategies run FIRST, then debate, SI, LLM opinion, psychology.
    pub async fn generate_signal_with_aggregation(
        &self,
        symbol: &str,
        current_price: f64,
        aggregated_signal: Option<&tredo_core::AggregatedSignal>,
    ) -> Result<Option<TradeSignal>, Box<dyn Error + Send + Sync>> {
        let rules = self.state.rules.read().await;
        let portfolio = self.state.portfolio.read().await;

        let bars = {
            let hist = self.state.ohlcv_history.read().await;
            hist.get(symbol).cloned().unwrap_or_default()
        };

        let context = MarketContext {
            symbol: symbol.to_string(),
            current_price,
            high: current_price * 1.01,
            low: current_price * 0.99,
            previous_close: current_price * 0.998,
            timestamp: Utc::now(),
            daily_pnl: portfolio.daily_pnl,
            equity: portfolio.cash_balance
                + portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.current_price * p.quantity)
                    .sum::<f64>(),
            consecutive_losses: portfolio.consecutive_losses,
            is_red_folder_day: false,
            trend_direction: None,
        };

        let pivots = calculate_pivot_points(
            context.high,
            context.low,
            context.previous_close,
            rules.pivot_method,
        );

        let reliance = aggregated_signal
            .map(|agg| (agg.conviction * 0.6 + agg.net_signal.abs() * 0.4).clamp(0.0, 1.0))
            .unwrap_or_else(|| calculate_confluence_score(&context, &pivots));
        let session = get_indian_session_info(Utc::now());

        let forecast_summary = {
            let last = self.state.last_forecast.read().await;
            match last.as_ref() {
                Some(v) => v["summary"]
                    .as_str()
                    .unwrap_or("No forecast summary")
                    .to_string(),
                None => "Kronos unavailable".to_string(),
            }
        };

        let market_regime = *self.state.market_regime.read().await;
        let trend_label = match market_regime {
            Some(MarketRegime::TrendingBull) => "Bullish",
            Some(MarketRegime::TrendingBear) => "Bearish",
            Some(MarketRegime::Ranging) => "Ranging",
            _ => "Neutral",
        };

        let portfolio_heat: f64 = {
            let total_risk: f64 = portfolio.open_positions.iter().map(|p| p.risk_amount).sum();
            if portfolio.total_equity > 0.0 {
                total_risk / portfolio.total_equity
            } else {
                0.0
            }
        };
        let consecutive_losses = portfolio.consecutive_losses;
        let daily_pnl_pct = portfolio.daily_pnl_pct;

        // ═══ STEP 0: Parallel tri-level check (rules ∥ LLM ∥ Kronos) ═════════
        // NOTE: Each layer now receives REAL data from SharedState:
        //   - Rules: real OHLCV bars for pivot calculation
        //   - LLM: real multi-TF/news/vector/patterns context (no more placeholder strings)
        //   - Kronos: full trajectory momentum analysis (not just last bar)
        // A 2-of-3 agreement gate is enforced: hard_agree=false → consensus="HOLD"
        let tri_verdict = crate::tri_level_validator::TriLevelValidator::new(self.state.clone())
            .run_parallel_check(
                symbol,
                current_price,
                reliance,
                trend_label,
                &forecast_summary,
                portfolio_heat,
                session.market_open,
                consecutive_losses,
            )
            .await;

        // ═══ Broadcast per-layer COT entries ═════════════════════════════════
        self.state
            .push_cot(
                "TriLevel::Rules",
                &format!("Rules layer for {} @ {:.2}", symbol, current_price),
                &tri_verdict.rules.action,
                &tri_verdict.rules.reasoning,
                tri_verdict.rules.confidence,
                0,
                None,
                Some(symbol.to_string()),
            )
            .await;
        self.state
            .push_cot(
                "TriLevel::LLM",
                &format!("LLM layer for {} @ {:.2}", symbol, current_price),
                &tri_verdict.llm.action,
                &tri_verdict.llm.reasoning,
                tri_verdict.llm.confidence,
                0,
                None,
                Some(symbol.to_string()),
            )
            .await;
        self.state
            .push_cot(
                "TriLevel::Kronos",
                &format!("Kronos layer for {} @ {:.2}", symbol, current_price),
                &tri_verdict.kronos.action,
                &tri_verdict.kronos.reasoning,
                tri_verdict.kronos.confidence,
                0,
                None,
                Some(symbol.to_string()),
            )
            .await;
        self.state
            .push_cot(
                "TriLevel::Consensus",
                &format!("Tri-level consensus for {} (agree={}/3 hard={})", symbol, tri_verdict.agreement_count, tri_verdict.hard_agree),
                &tri_verdict.consensus_action,
                &format!(
                    "weighted_signal={:.3} agree={}/{} hard_agree={} unanimous={}",
                    tri_verdict.consensus_signal,
                    tri_verdict.agreement_count,
                    3,
                    tri_verdict.hard_agree,
                    tri_verdict.direction_unanimous,
                ),
                tri_verdict.consensus_signal.abs().clamp(0.0, 1.0),
                0,
                None,
                Some(symbol.to_string()),
            )
            .await;

        // ═══ STEP 1: Compute indicators ═══════════════════════════════════════
        let rsi = crate::helpers::compute_rsi(&bars, 14);
        let (_, _, macd_hist) = crate::helpers::compute_macd(&bars);
        let atr_pct = {
            if bars.len() >= 14 {
                let mut tr_sum = 0.0;
                for bar in bars.iter().skip(1) {
                    let tr = (bar.high - bar.low).abs();
                    tr_sum += tr;
                }
                tr_sum / bars.len() as f64 / current_price
            } else {
                match trend_label {
                    "Bullish" => 0.015,
                    "Bearish" => 0.018,
                    "Ranging" => 0.012,
                    _ => 0.025,
                }
            }
        };

        // Patterns & volume from MI
        let patterns_context = {
            let pats = self.state.last_patterns.read().await;
            match pats.get(symbol) {
                Some(p) if !p.is_empty() => tredo_core::format_patterns(p),
                _ => String::new(),
            }
        };

        let patterns_for_levels: Vec<tredo_core::CandlestickPattern> = {
            let p = self.state.last_patterns.read().await;
            p.get(symbol).cloned().unwrap_or_default()
        };

        // Pull metrics snapshot
        let (news_ctx, meter) = {
            let n = self.state.latest_news.read().await;
            let m = self.state.latest_metrics.read().await;
            (n.get(symbol).cloned(), m.get(symbol).cloned())
        };
        let meter_atr = meter.as_ref().map(|m| m.atr_pct).unwrap_or(atr_pct);
        if let Some(m) = &meter {
            println!(
                "[Strategy] using meter snapshot: rsi={:.1} conf={:.2} regime={}",
                m.rsi_14, m.confluence_hint, m.regime_hint
            );
        }

        // ═══ STEP 2: Run DETERMINISTIC STRATEGIES FIRST (no LLM) ══════════════
        // These are the PRIMARY decision makers. No LLM needed.
        let (supports, resistances) = crate::helpers::compute_support_resistance(&bars, 50);
        let regime = market_regime.unwrap_or(MarketRegime::Ranging);

        let det_strategy_result = crate::deterministic_strategies::select_best_strategy(
            &bars,
            current_price,
            &regime,
            &supports,
            &resistances,
        );

        // Default levels from autonomous calculation (fallback)
        let (auto_entry, auto_stop_loss, auto_take_profit, _auto_rr) =
            crate::helpers::compute_autonomous_levels(
                symbol,
                current_price,
                &pivots,
                &patterns_for_levels,
                regime,
                rsi,
                macd_hist,
                meter_atr,
                &rules,
                aggregated_signal,
                None,
            );

        // If deterministic strategy fired, use its levels as the PRIMARY signal
        let (mut primary_action, mut primary_conf, primary_entry, primary_sl, primary_tp);
        let mut strategy_source = "deterministic";

        // Tri-level BLOCK from rules layer overrides everything
        if tri_verdict.rules.action == "BLOCK" {
            println!(
                "[Strategy] Tri-level rules BLOCK for {} — skipping trade",
                symbol
            );
            return Ok(None);
        }

        // 2-of-3 agreement gate: if hard_agree=false, tri-level cannot confirm any trade
        // This is logged but does not block deterministic strategies (they run below)
        if !tri_verdict.hard_agree && tri_verdict.consensus_action != "HOLD" {
            println!(
                "[Strategy] ⚠ {}: tri-level agreement gate weak ({}/3) — deterministic may still fire",
                symbol, tri_verdict.agreement_count
            );
        }

        // ═══ FAST-PATH (Fix #4): High-confidence deterministic strategy ═══
        // If deterministic strategy fires with confidence > 0.7 and
        // confluence > 0.5, skip debate/SI/LLM entirely. The deterministic
        // strategy is already correct — no need for more layers.
        if let Some(ref det) = det_strategy_result {
            let high_conf_fast_path = det.confidence > 0.7 && reliance > 0.5;

            primary_action = if det.direction == tredo_core::TradeDirection::Long {
                "BUY"
            } else {
                "SELL"
            };
            primary_conf = det.confidence;
            primary_entry = det.entry_price;
            primary_sl = det.stop_loss;
            primary_tp = det.take_profit;

            if high_conf_fast_path {
                println!(
                    "[Strategy] 🚀 FAST-PATH: {} @ entry={:.2} SL={:.2} TP={:.2} (conf={:.1}%) — skipping debate/SI/LLM",
                    det.strategy_name, primary_entry, primary_sl, primary_tp, primary_conf * 100.0
                );
                // Build signal directly without debate/SI/LLM overhead
                // Jump to signal construction (avoids the entire STEP 3-5 code below)
                // We do this by setting strategy_source and skipping to signal build
                strategy_source = "deterministic_fast";
                // The let binding `si_final_action` etc. will be set at the end
            } else {
                println!(
                    "[Strategy] 🎯 Deterministic strategy FIRED: {} @ entry={:.2} SL={:.2} TP={:.2} (conf={:.1}%) — continuing with debate/SI/LLM",
                    det.strategy_name, primary_entry, primary_sl, primary_tp, primary_conf * 100.0
                );
            }
        } else {
            // Fallback: use autonomous levels + debate (still no LLM)
            primary_action = "HOLD";
            primary_conf = 0.5;
            primary_entry = auto_entry;
            primary_sl = auto_stop_loss;
            primary_tp = auto_take_profit;
            strategy_source = "debate";

            println!("[Strategy] No deterministic strategy fired — falling back to debate + autonomous levels");
        }

        // ═══ FAST-PATH SKIP: Skip debate/SI/LLM if deterministic was high-conf ══
        let mut skip_debate_si_llm = false;
        if let Some(ref det) = det_strategy_result {
            if det.confidence > 0.7 && reliance > 0.5 {
                skip_debate_si_llm = true;
            }
        }

        // ═══ STEP 3: Run debate layer (advisory, non-LLM multi-agent) ════════
        let aggregated_signal = {
            let agg = self.state.last_aggregated_signal.read().await;
            agg.clone()
        };

        let (debate_action, debate_conf, _debate_reason, _turns) = if skip_debate_si_llm {
            // Fast-path: skip debate entirely — just mark as APPROVE
            ("APPROVE".to_string(), primary_conf, String::new(), vec![])
        } else {
            let debate_input = AgentInput::ConfluenceRequest {
                context: context.clone(),
            };
            let result = crate::debate::run_debate(
                self.state.clone(),
                &debate_input,
                aggregated_signal.as_ref(),
            )
            .await;
            (result.0.to_string(), result.1, result.2, result.3)
        };

        // If deterministic strategy fired, debate is advisory only
        // If no deterministic strategy, debate becomes the primary signal
        if det_strategy_result.is_none() && debate_action != "HOLD" {
            primary_action = &debate_action;
            primary_conf = debate_conf;
            strategy_source = "debate";
        }

        // Tri-level consensus can upgrade HOLD → action when all layers agree
        if primary_action == "HOLD"
            && tri_verdict.consensus_action != "HOLD"
            && tri_verdict.consensus_signal.abs() > 0.35
            && tri_verdict.hard_agree // only upgrade if 2+ layers agree
        {
            if tri_verdict.consensus_action == "BUY" || tri_verdict.consensus_action == "SELL" {
                primary_action = tri_verdict.consensus_action.as_str();
            }
            primary_conf = tri_verdict.consensus_signal.abs().clamp(0.35, 0.85);
            strategy_source = "tri_level";
            println!(
                "[Strategy] Tri-level consensus upgraded HOLD → {} (signal={:.2}, agree={}/3)",
                primary_action, tri_verdict.consensus_signal, tri_verdict.agreement_count
            );
        }

        // ── Direction lock: when tri-level hard_agrees on a direction that contradicts
        // the deterministic strategy, log a DIRECTION_CONFLICT and prefer tri-level
        // if it is unanimous (all 3 layers agree) and the deterministic is not high-conf.
        if tri_verdict.hard_agree
            && tri_verdict.consensus_action != "HOLD"
            && primary_action != "HOLD"
            && primary_action != tri_verdict.consensus_action
        {
            if tri_verdict.direction_unanimous {
                // All 3 layers disagree — defer to tri-level
                println!(
                    "[Strategy] ⚠ DIRECTION_CONFLICT for {}: deterministic={} but tri-level unanimous={} — deferring to tri-level",
                    symbol, primary_action, tri_verdict.consensus_action
                );
                self.state
                    .push_cot(
                        "StrategyDecision",
                        &format!("Direction lock for {} — unanimous tri-level override", symbol),
                        "DIRECTION_LOCKED",
                        &format!(
                            "Deterministic={} overridden by unanimous tri-level={}. agree={}/3",
                            primary_action, tri_verdict.consensus_action, tri_verdict.agreement_count
                        ),
                        0.0,
                        0,
                        None,
                        Some(symbol.to_string()),
                    )
                    .await;
                primary_action = tri_verdict.consensus_action.as_str();
                primary_conf = tri_verdict.consensus_signal.abs().clamp(0.45, 0.80);
                strategy_source = "tri_level_lock";
            } else {
                // 2/3 agree — log conflict but let deterministic proceed with reduced confidence
                println!(
                    "[Strategy] ⚠ DIRECTION_CONFLICT for {}: deterministic={} but tri-level(2/3)={} — keeping deterministic at reduced conf",
                    symbol, primary_action, tri_verdict.consensus_action
                );
                self.state
                    .push_cot(
                        "StrategyDecision",
                        &format!("Direction conflict for {} (2/3 tri disagrees)", symbol),
                        "DIRECTION_CONFLICT_2OF3",
                        &format!(
                            "Deterministic={} conflicts with tri-level(2/3)={}. Keeping det. at 0.75x conf.",
                            primary_action, tri_verdict.consensus_action
                        ),
                        0.0,
                        0,
                        None,
                        Some(symbol.to_string()),
                    )
                    .await;
                primary_conf *= 0.75; // reduce confidence due to conflict
            }
        }

        // Direction is computed AFTER SuperIntelligence adjusts the action (see below)

        // Vector memory context
        let vector_context = {
            let vm = self.state.vector_memory.read().await;
            if !vm.is_empty() {
                let query = format!(
                    "{} regime={} confluence={:.2} price={:.2}",
                    symbol, trend_label, reliance, current_price
                );
                match vm.search(&query, 3, &self.state.llm).await {
                    Ok(results) if !results.is_empty() => {
                        let mut lines = vec!["Vector memory regime matches:".to_string()];
                        for r in &results {
                            let regret = r
                                .regret_score
                                .map(|s| format!(" regret={:.2}", s))
                                .unwrap_or_default();
                            lines.push(format!(
                                "  {} (sim={:.0}%){}",
                                r.summary_text,
                                r.similarity * 100.0,
                                regret
                            ));
                        }
                        lines.join(" | ")
                    }
                    _ => "No strong vector memory regime matches.".to_string(),
                }
            } else {
                "Vector memory empty (JSON fallback or no episodes yet).".to_string()
            }
        };

        // Build technical reasoning
        let tech_reasoning = format!(
            "Source: {} | {} | RSI={:.1} MACD_hist={:.4} ATR%={:.2}% | Pivots R1/S1={:.2}/{:.2} | Patterns: {} | Debate: {} (conf {:.2}) | {}",
            strategy_source,
            if primary_action == "BUY" { "BUY" } else { if primary_action == "SELL" { "SELL" } else { "HOLD" } },
            rsi, macd_hist, atr_pct * 100.0,
            pivots.r1, pivots.s1,
            patterns_context,
            debate_action, debate_conf,
            vector_context
        );

        // ═══ FAST-PATH SKIP SI/LLM ═══════════════════════════════════════
        // If deterministic strategy fired with high confidence, skip all
        // expensive layers (SuperIntelligence, LLM) and jump directly to
        // signal construction. This dramatically increases trade throughput.
        if skip_debate_si_llm {
            // Skip SI and LLM entirely — deterministic strategy is already solid
            let signal = TradeSignal {
                symbol: symbol.to_string(),
                direction: if primary_action == "BUY" { tredo_core::TradeDirection::Long } else { tredo_core::TradeDirection::Short },
                entry_price: primary_entry,
                stop_loss: primary_sl,
                take_profit: primary_tp,
                position_size: 0.0, // computed below
                confidence_score: primary_conf.min(0.95),
                confluence_score: reliance,
                risk_reward_ratio: ((primary_tp - primary_entry).abs() / (primary_entry - primary_sl).abs()).max(1.0),
                reasoning: format!("FAST-PATH: {} (conf={:.1}%) | {}", strategy_source, primary_conf * 100.0, tech_reasoning),
                timestamp: Utc::now(),
                session_valid: session.market_open,
                risk_check_passed: true,
            };
            // Apply psychology sizing
            let (equity, _fresh_heat, _fresh_consecutive_losses) = {
                let p = self.state.portfolio.read().await;
                (p.cash_balance + p.open_positions.iter().map(|pos| pos.current_price * pos.quantity).sum::<f64>(), 0.0, p.consecutive_losses)
            };
            let effective_risk = (rules.max_risk_per_trade * 1.0).max(0.002);
            let kelly_stats = self.state.episode_store.kelly_trade_stats(50);
            let (position_size, _kelly_half) = crate::helpers::kelly_capped_position_size(
                equity, effective_risk, signal.entry_price, signal.stop_loss, &kelly_stats,
            );
            let cash_available = { self.state.portfolio.read().await.cash_balance };
            let max_from_equity = equity * 0.04;
            let max_from_cash = cash_available * 0.95;
            let max_per_symbol = max_from_equity.min(max_from_cash) * 0.98; // 2% buffer for slippage & rounding
            let final_size = if position_size * signal.entry_price > max_per_symbol {
                max_per_symbol / signal.entry_price.max(0.0001)
            } else {
                position_size
            };
            let mut signal = signal;
            signal.position_size = final_size;
            // Discipline gate
            let discipline = validate_trade_setup(&context, &rules);
            if !discipline.passed {
                println!("[StrategyDecisionAgent] FAST-PATH signal rejected by DisciplinedCore");
                return Ok(None);
            }
            println!(
                "[StrategyDecisionAgent] 🚀 FAST-PATH: {} {} @ entry={:.2} SL={:.2} TP={:.2} (RR {:.1}:1, conf {:.1}%)",
                if signal.direction == tredo_core::TradeDirection::Long { "BUY" } else { "SELL" },
                symbol, signal.entry_price, signal.stop_loss, signal.take_profit,
                signal.risk_reward_ratio, signal.confidence_score * 100.0
            );
            return Ok(Some(signal));
        }

        // ═══ STEP 4: SuperIntelligence cross-validation ══════════════════════
        let mut si_final_action = primary_action.to_string();
        let mut si_final_conf = primary_conf;
        let mut si_final_entry = primary_entry;
        let mut si_final_sl = primary_sl;
        let mut si_final_tp = primary_tp;
        let mut si_reasoning = tech_reasoning.clone();

        if let Some(ref agg_signal) = aggregated_signal {
            let mut si_evidence = crate::debate::EvidenceBuilder::new(trend_label);
            si_evidence.add("rsi", (50.0 - rsi) / 50.0, 0.15);
            si_evidence.add("macd", macd_hist * 5.0, 0.10);
            si_evidence.add("confluence", (reliance - 0.5) * 2.0, 0.20);
            if let Some(ref reg) = *self.state.market_regime.read().await {
                let reg_score = match reg {
                    MarketRegime::TrendingBull => 0.5,
                    MarketRegime::TrendingBear => -0.5,
                    _ => 0.0,
                };
                si_evidence.add("regime", reg_score, 0.20);
            }

            let si_result = crate::super_intelligence::SuperIntelligence::analyze(
                &self.state,
                symbol,
                current_price,
                agg_signal,
                &si_evidence,
                &si_final_action,
                si_final_conf,
            )
            .await;

            let si_action = si_result.recommended_action.clone();
            let si_conf = si_result.recommended_confidence;
            let si_conviction = si_result.conviction.final_conviction;
            let si_validation = si_result.validation.overall_validation_score;
            let si_trace = si_result.decision_trace.format_for_log();
            let si_should_proceed = si_result.should_proceed;

            si_reasoning = format!(
                "{} | SI: {} (conf {:.1}%) | Conviction: {:.1}% | Validation: {:.1}% | {}",
                si_reasoning,
                si_action,
                si_conf * 100.0,
                si_conviction * 100.0,
                si_validation * 100.0,
                si_trace
            );

            si_final_action = si_action;
            si_final_conf = si_conf;

            // ═══ STEP 5: LLM as cross-check opinion only ══════════════════
            // LLM is consulted but cannot override the deterministic + SI result.
            // It provides an opinion that is logged and factored into reasoning.
            let calendar_context = {
                let cal = self.state.calendar_events.read().await;
                if cal.is_empty() {
                    "No high-impact events scheduled.".to_string()
                } else {
                    cal.iter()
                        .map(|e| {
                            format!(
                                "⚠ {} at {} ({:?})",
                                e.title,
                                e.time.as_deref().unwrap_or("TBD"),
                                e.impact
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            };
            let (trading_mode, daily_goal_context) = {
                let goals = self.state.trading_goals.read().await;
                (
                    format!("{:?}", goals.mode),
                    format!(
                        "Daily P&L target: {:.1}% | Current: {:.2}%",
                        goals.daily_target_pnl_pct * 100.0,
                        daily_pnl_pct * 100.0
                    ),
                )
            };
            // Enhanced MTF context: now includes full analysis from all 11 timeframes
            let multi_tf_context = {
                let mtf_agg = self.state.multi_tf_aggregate.read().await;
                if let Some(agg) = mtf_agg.get(symbol) {
                    format!(
                        "MTF: {} (signal={:.3}, agree={:.0}%, tfs={}) | {}",
                        agg.aggregate_direction,
                        agg.aggregate_signal,
                        agg.agreement_pct * 100.0,
                        agg.tf_count,
                        agg.tf_analyses
                            .iter()
                            .map(|(tf, a)| format!(
                                "{}: dir={} conf={:.1}% rsi={:.0}",
                                tf,
                                a.aggregated_direction,
                                a.aggregated_conviction * 100.0,
                                a.metrics.rsi_14
                            ))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    )
                } else {
                    let mtf = self.state.multi_timeframe_data.read().await;
                    if let Some(tf_data) = mtf.get(symbol) {
                        tf_data
                            .iter()
                            .map(|tf| {
                                let bars_summary = if tf.ohlcv.len() >= 2 {
                                    let last = tf.ohlcv.last().unwrap();
                                    let prev = &tf.ohlcv[tf.ohlcv.len() - 2];
                                    let chg = (last.close - prev.close) / prev.close * 100.0;
                                    format!("close={:.2} ({:+.2}%)", last.close, chg)
                                } else {
                                    "no data".to_string()
                                };
                                format!("{}: {}", tf.timeframe, bars_summary)
                            })
                            .collect::<Vec<_>>()
                            .join(" | ")
                    } else {
                        "No multi-TF data available.".to_string()
                    }
                }
            };
            let agent_market_summary = {
                let s = self.state.agent_market_summary.read().await;
                if s.is_empty() {
                    "No market summary yet.".to_string()
                } else {
                    s.clone()
                }
            };
            let news_context = match &news_ctx {
                Some(ctx) => ctx.to_prompt_string(),
                None => "No recent news.".to_string(),
            };

            // LLM decision (opinion only) — Fix #6: Hard 5-second timeout
            let llm_decision: Option<LlmTradeDecision> = {
                self.state.push_live_comm(
                    "Verifier",
                    "Ollama",
                    "QUERY",
                    &format!("Requesting LLM opinion for {} @ {:.2} (Model: {})", symbol, current_price, self.state.config.llm_model),
                    Some(symbol.to_string()),
                ).await;

                let llm_future = self
                    .state
                    .llm
                    .ask_for_trade_decision(
                        symbol,
                        current_price,
                        reliance,
                        trend_label,
                        pivots.pivot,
                        pivots.r1,
                        pivots.s1,
                        &forecast_summary,
                        portfolio_heat,
                        session.market_open,
                        consecutive_losses,
                        &calendar_context,
                        &trading_mode,
                        &daily_goal_context,
                        &multi_tf_context,
                        &agent_market_summary,
                        &news_context,
                        &vector_context,
                        &patterns_context,
                    );

                // ═══ HARD 25-SECOND LLM TIMEOUT ════════════════════
                // Prevent LLM from blocking the pipeline if it's slow or hung.
                // If timeout elapses, fall through to deterministic path.
                let decision = tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    llm_future,
                )
                .await
                .unwrap_or_else(|_| {
                    println!("[Strategy] ⏱ LLM timed out after 25s for {} — using deterministic-only path", symbol);
                    tredo_core::LlmTradeDecision {
                        action: "HOLD".to_string(),
                        reason: "LLM timeout (25s)".to_string(),
                        entry: 0.0,
                        sl: 0.0,
                        tp: 0.0,
                    }
                });

                let available = decision.action != "HOLD" && !decision.reason.contains("Parse failed") && !decision.reason.contains("unavailable");

                if available {
                    self.state.push_live_comm(
                        "Ollama",
                        "Verifier",
                        &decision.action,
                        &format!("Opinion: {}", decision.reason),
                        Some(symbol.to_string()),
                    ).await;
                    println!(
                        "[Strategy] 🤖 LLM opinion for {}: {} (SI says {}) — cross-check only",
                        symbol, decision.action, si_final_action
                    );
                    Some(decision)
                } else {
                    self.state.push_live_comm(
                        "Ollama",
                        "Verifier",
                        "HOLD",
                        &format!("Opinion: LLM unavailable/HOLD: {}", decision.reason),
                        Some(symbol.to_string()),
                    ).await;
                    println!("[Strategy] ⚠ LLM unavailable or HOLD — deterministic path used without LLM");
                    None
                }
            };

            // LLM cannot override — only logs its opinion
            if let Some(ref llm) = llm_decision {
                if llm.action != si_final_action {
                    println!(
                        "[Strategy] ⚖️ LLM disagrees with SI: LLM={}, SI={} — deferring to SI (primary)",
                        llm.action, si_final_action
                    );
                } else {
                    println!(
                        "[Strategy] ✅ LLM confirms SI decision: both agree on {}",
                        llm.action
                    );
                }
                // LLM can refine levels if SI also approves
                if llm.action == si_final_action && si_should_proceed {
                    si_final_entry = (si_final_entry + llm.entry) / 2.0;
                    si_final_sl = (si_final_sl + llm.sl) / 2.0;
                    si_final_tp = (si_final_tp + llm.tp) / 2.0;
                }
            }
        }

        // ═══ MTF AGGREGATE CONFIDENCE BOOST ════════════════════════════════
        // If multiple timeframes agree on the same direction, boost the confidence.
        // If they disagree strongly, reduce confidence (more uncertainty).
        let (mtf_boost, mtf_agreement_pct) = {
            let mtf_agg = self.state.multi_tf_aggregate.read().await;
            match mtf_agg.get(symbol) {
                Some(agg) if agg.tf_count >= 3 => {
                    // Count how many TFs agree with the SI final direction
                    let target_dir = if si_final_action == "BUY" {
                        "bullish"
                    } else if si_final_action == "SELL" {
                        "bearish"
                    } else {
                        ""
                    };
                    if !target_dir.is_empty() {
                        let agreed = agg
                            .tf_analyses
                            .iter()
                            .filter(|(_, a)| a.aggregated_direction == target_dir)
                            .count();
                        let total = agg.tf_analyses.len().max(1);
                        let pct = agreed as f64 / total as f64;
                        // Boost: strong agreement (60%+) adds up to +0.08, low agreement (<30%) reduces by -0.05
                        let boost = if pct >= 0.6 {
                            (pct - 0.5) * 0.20 // 0.6→0.02, 1.0→0.10
                        } else if pct <= 0.3 {
                            -0.05 // strong disagreement = uncertainty
                        } else {
                            0.0
                        };
                        (boost, pct)
                    } else {
                        (0.0, 0.0)
                    }
                }
                _ => (0.0, 0.0),
            }
        };
        if mtf_boost != 0.0 {
            println!(
                "[Strategy] 🎯 MTF confidence boost: {:+.4} (agreement={:.0}%)",
                mtf_boost,
                mtf_agreement_pct * 100.0
            );
        }

        let final_conf = (si_final_conf + mtf_boost).clamp(0.0, 0.98);
        let final_reasoning = if mtf_boost.abs() > 0.01 {
            format!(
                "{} | MTF: {:.0}% of {} TFs agree, boost={:+.2}",
                si_reasoning,
                mtf_agreement_pct * 100.0,
                symbol,
                mtf_boost
            )
        } else {
            si_reasoning
        };
        let (final_action, final_entry, final_sl, final_tp) =
            (si_final_action, si_final_entry, si_final_sl, si_final_tp);

        // Store final reasoning for UI display
        {
            let mut last_reason = self.state.last_llm_reason.write().await;
            *last_reason = final_reasoning.clone();
        }

        if final_action == "HOLD" || final_conf < 0.35 {
            println!(
                "[StrategyDecisionAgent] HOLD for {} (rsi={:.1} macd_hist={:.4} source={})",
                symbol, rsi, macd_hist, strategy_source
            );
            return Ok(None);
        }

        // Finalize levels
        let signal_entry = if final_entry > 0.0 {
            final_entry
        } else {
            auto_entry
        };
        let signal_sl = if final_sl > 0.0 {
            final_sl
        } else {
            auto_stop_loss
        };
        let signal_tp = if final_tp > 0.0 {
            final_tp
        } else {
            auto_take_profit
        };

        let final_rr = {
            let risk = (signal_entry - signal_sl).abs();
            let reward = (signal_tp - signal_entry).abs();
            if risk > 0.0 {
                reward / risk
            } else {
                2.0
            }
        };

        // ═══ STEP 6: Behavioral Psychology Sizing Adjustment ═════════════════
        // Get psychology-adjusted position size multiplier
        let (equity, fresh_heat, fresh_consecutive_losses) = {
            let p = self.state.portfolio.read().await;
            let eq = p.cash_balance
                + p.open_positions
                    .iter()
                    .map(|pos| pos.current_price * pos.quantity)
                    .sum::<f64>();
            let total_risk: f64 = p.open_positions.iter().map(|pos| pos.risk_amount).sum();
            let heat = if p.total_equity > 0.0 {
                total_risk / p.total_equity
            } else {
                0.0
            };
            (eq, heat, p.consecutive_losses)
        };

        // Record the decision in behavioral psychology engine
        {
            let mut psych = self.state.behavioral_psychology.write().await;
            psych.record_decision(&final_action, final_conf, signal_entry);
        }

        // Analyze psychological state
        let psych_snapshot = {
            let psych = self.state.behavioral_psychology.read().await;
            psych.analyze(
                fresh_heat,
                0.0, // drawdown_pct — pipeline doesn't track this directly, leave 0
                0,   // total_trades_today — tracked in portfolio
                0.0, // daily_pnl_pct — already weighted in heat
                equity, 100_000.0, // initial_equity from config
            )
        };
        let psych_size_mult = psych_snapshot.position_size_multiplier;

        if psych_size_mult < 1.0 {
            println!(
                "[Strategy] 🧠 Behavioral Psychology: size_mult={:.2}x (state={})",
                psych_size_mult, psych_snapshot.emotional_state.state_label
            );
            if !psych_snapshot.active_biases.is_empty() {
                for bias in &psych_snapshot.active_biases {
                    println!(
                        "[Strategy] ⚠ Bias detected: {} ({:.0}%) — {}",
                        bias.bias.name(),
                        bias.severity * 100.0,
                        bias.corrective_action
                    );
                }
            }
        }

        // Compute adaptive risk multiplier
        let adaptive_risk_mult = {
            let conf_mult = (final_conf / 0.7).clamp(0.5, 1.2);
            let loss_mult = if fresh_consecutive_losses >= 3 {
                0.5
            } else if fresh_consecutive_losses >= 2 {
                0.7
            } else {
                1.0
            };
            let heat_mult = if fresh_heat > 0.08 {
                0.5
            } else if fresh_heat > 0.05 {
                0.7
            } else {
                1.0
            };
            let regime_mult = match trend_label {
                "Bullish" => 1.0,
                "Bearish" => 0.7,
                "Ranging" => 0.8,
                _ => 0.6,
            };
            (conf_mult * loss_mult * heat_mult * regime_mult).clamp(0.3, 1.2)
        };

        // Apply BOTH adaptive risk AND psychological sizing
        let effective_risk =
            (rules.max_risk_per_trade * adaptive_risk_mult * psych_size_mult).max(0.002);
        let kelly_stats = self.state.episode_store.kelly_trade_stats(50);
        let (position_size, kelly_half) = crate::helpers::kelly_capped_position_size(
            equity,
            effective_risk,
            signal_entry,
            signal_sl,
            &kelly_stats,
        );
        if let Some(hk) = kelly_half {
            println!(
                "[StrategyDecision] Kelly half={:.1}% ({} trades, win={:.0}%) → size capped",
                hk * 100.0,
                kelly_stats.trade_count,
                kelly_stats.win_probability * 100.0
            );
        }

        // Validate against cash and 1/25 capital allocation rule
        let cash_available = { self.state.portfolio.read().await.cash_balance };
        let max_from_equity = equity * 0.04;
        let max_from_cash = cash_available * 0.95;
        let max_per_symbol = max_from_equity.min(max_from_cash) * 0.98; // 2% buffer for slippage & rounding
        let final_position_size = if position_size * signal_entry > max_per_symbol {
            max_per_symbol / signal_entry.max(0.0001)
        } else {
            position_size
        };

        // Recompute direction from SI-adjusted final action
        let direction = if final_action == "BUY" {
            tredo_core::TradeDirection::Long
        } else if final_action == "SELL" {
            tredo_core::TradeDirection::Short
        } else {
            if final_entry > current_price {
                tredo_core::TradeDirection::Long
            } else {
                tredo_core::TradeDirection::Short
            }
        };

        let signal = TradeSignal {
            symbol: symbol.to_string(),
            direction,
            entry_price: signal_entry,
            stop_loss: signal_sl,
            take_profit: signal_tp,
            position_size: final_position_size,
            confidence_score: final_conf.min(0.95),
            confluence_score: reliance,
            risk_reward_ratio: final_rr,
            reasoning: final_reasoning,
            timestamp: Utc::now(),
            session_valid: session.market_open,
            risk_check_passed: true,
        };

        // ── Geometry Consistency Gate (3-level cross-check) ──────────────────
        // Block execution if the signal direction contradicts a hard-agreed tri-level consensus.
        // This is the final safety net: even after direction lock above, verify the signal.
        if let Err(conflict) = crate::tri_level_validator::is_geometry_consistent(&tri_verdict, &signal) {
            println!(
                "[StrategyDecisionAgent] ❌ GEOMETRY_CONFLICT for {} — aborting: {}",
                symbol, conflict
            );
            self.state
                .push_cot(
                    "StrategyDecision",
                    &format!("Geometry conflict check for {}", symbol),
                    "BLOCKED_GEOMETRY_CONFLICT",
                    &conflict,
                    0.0,
                    0,
                    None,
                    Some(symbol.to_string()),
                )
                .await;
            return Ok(None);
        }

        // Discipline gate
        let discipline = validate_trade_setup(&context, &rules);
        if !discipline.passed {
            println!("[StrategyDecisionAgent] Signal rejected by DisciplinedCore");
            return Ok(None);
        }

        println!(
            "[StrategyDecisionAgent] {} {} @ entry={:.2} SL={:.2} TP={:.2} (RR {:.1}:1, conf {:.1}%, source={})",
            if direction == tredo_core::TradeDirection::Long { "BUY" } else { "SELL" },
            symbol, signal_entry, signal_sl, signal_tp, final_rr,
            signal.confidence_score * 100.0, strategy_source
        );

        Ok(Some(signal))
    }
}
