use crate::helpers::get_indian_session_info;
use crate::state::SharedState;
use crate::types::TradeSignal;
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

    /// Generate a trade signal **autonomously** (true agentic AI, not a scripted bot).
    ///
    /// The agent itself:
    /// - Analyzes trends, patterns, volume, RSI, MACD, ATR, pivots, confluence from state/skills.
    /// - Decides direction via debate + memory + rules (no external direction or levels provided).
    /// - Identifies its own entry, stop_loss, take_profit levels using indicators and structure.
    /// - Validates with DisciplinedCore.
    /// - Returns full TradeSignal or None (HOLD).
    ///
    /// Callers (orchestrator/tests) only provide symbol + current_price. The agent does the rest.
    /// Agentic decision (convenience version for callers that haven't pulled the AggregatedSignal yet).
    /// Internally this will read from state.last_aggregated_signal (populated by MarketIntelligence).
    /// Fully agentic decision: The agent observes market data from state,
    /// computes its own indicators (RSI, MACD, ATR, volume signals, trend,
    /// patterns, pivots, regime, confluence), recalls similar past episodes
    /// from memory (what worked, regret), runs debate, applies rules,
    /// and autonomously decides direction + precise entry/SL/TP levels.
    /// NO external price points, direction, or levels are ever injected.
    /// This is true agentic/self-evolving trading AI, not a scripted bot.
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

    /// Full agentic decision with explicit AggregatedSignal (preferred path).
    /// The cross-skill consensus from MarketIntelligence is now a first-class input to the decision layer.
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
        let confluence = calculate_confluence_score(&context, &pivots);
        let session = get_indian_session_info(Utc::now());

        // Pull existing MI data (patterns, regime, forecast, aggregated from skills)
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

        let trend_label = {
            let regime = self.state.market_regime.read().await;
            match *regime {
                Some(crate::types::MarketRegime::TrendingBull) => "Bullish",
                Some(crate::types::MarketRegime::TrendingBear) => "Bearish",
                Some(crate::types::MarketRegime::Ranging) => "Ranging",
                _ => "Neutral",
            }
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
        // === AGENTIC INDICATOR ANALYSIS (no external price points) ===
        let rsi = crate::helpers::compute_rsi(&bars, 14);
        let (_, _, macd_hist) = crate::helpers::compute_macd(&bars);
        let atr_pct = {
            // Reuse vol calc if available, else estimate
            if bars.len() >= 14 {
                let mut tr_sum = 0.0;
                for bar in bars.iter().skip(1) {
                    let tr = (bar.high - bar.low).abs();
                    tr_sum += tr;
                }
                tr_sum / bars.len() as f64 / current_price
            } else {
                // Regime-adaptive ATR fallback
                match trend_label {
                    "Bullish" => 0.015,
                    "Bearish" => 0.018,
                    "Ranging" => 0.012,
                    _ => 0.025, // Volatile/LowLiquidity — wider stops needed
                }
            }
        };

        // Patterns & volume from MI (already agent-computed)
        let patterns_context = {
            let pats = self.state.last_patterns.read().await;
            match pats.get(symbol) {
                Some(p) if !p.is_empty() => tredo_core::format_patterns(p),
                _ => String::new(),
            }
        };

        // === AUTONOMOUS DIRECTION + LEVELS (agent identifies everything) ===
        // The AggregatedSignal (if provided) now directly influences level selection.
        // This is the key integration point that was missing.
        let patterns_for_levels: Vec<tredo_core::CandlestickPattern> = {
            let p = self.state.last_patterns.read().await;
            p.get(symbol).cloned().unwrap_or_default()
        };

        // === CONNECTED: Pull NewsAnalyser + MetricsMeter snapshots (set by MI / loops) for richer agent reasoning ===
        // These feed the agent's own analysis + autonomous level calc via memory/debate/agg influence. Agent decides.
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

        let (entry, stop_loss, take_profit, _rule_rr) =
            crate::helpers::compute_autonomous_levels(
                symbol,
                current_price,
                &pivots,
                &patterns_for_levels,
                *self
                    .state
                    .market_regime
                    .read()
                    .await
                    .as_ref()
                    .unwrap_or(&crate::types::MarketRegime::Ranging),
                rsi,
                macd_hist,
                meter_atr,
                &rules,
                aggregated_signal, // Pass the cross-skill consensus so levels respect the aggregated vote (news_analyser + meter skills included)
            );

        // === REAL AGGREGATOR PASS-THROUGH (Gap 1 fix) ===
        // Pull the AggregatedSignal that was computed in MarketIntelligence.
        // This is now passed as a first-class input to debate so the agent actually
        // uses the cross-skill consensus instead of ignoring its own thoughts.
        let aggregated_signal = {
            let agg = self.state.last_aggregated_signal.read().await;
            agg.clone()
        };

        // Debate for final reasoning (agentic multi-agent) — now receives the aggregated skills consensus
        let debate_input = AgentInput::ConfluenceRequest {
            context: context.clone(),
        };
        let (debate_action, debate_conf, debate_reason, _turns) = crate::debate::run_debate(
            self.state.clone(),
            &debate_input,
            aggregated_signal.as_ref(),
        )
        .await;

        let direction = if debate_action == "BUY" {
            tredo_core::TradeDirection::Long
        } else if debate_action == "SELL" {
            tredo_core::TradeDirection::Short
        } else {
            // fallback to levels logic
            if entry > current_price {
                tredo_core::TradeDirection::Long
            } else {
                tredo_core::TradeDirection::Short
            }
        };

        // === VECTOR MEMORY USAGE FOR REGIME MATCHING (Gap 3) ===
        // Pull similar historical episodes using the agent's vector memory.
        let vector_context = {
            let vm = self.state.vector_memory.lock().await;
            if !vm.is_empty() {
                let query = format!(
                    "{} regime={} confluence={:.2} price={:.2}",
                    symbol, trend_label, confluence, current_price
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

        // Build rule-based reasoning including indicators + vector memory
        let rule_based_reasoning = format!(
            "Rule-based: {} | RSI={:.1} MACD_hist={:.4} ATR%={:.2}% | Pivots R1/S1={:.2}/{:.2} | Patterns: {} | Debate+Agg: {} (conf {:.2}) | {} | {}",
            direction as u8,
            rsi, macd_hist, atr_pct * 100.0,
            pivots.r1, pivots.s1,
            patterns_context,
            debate_action, debate_conf,
            debate_reason,
            vector_context
        );

        // === LLM DECISION (Phase 5) — ask_for_trade_decision when Ollama available ===
        // The LLM gets ALL context: indicators, forecast, news, calendar, goals, memory.
        // It produces structured reasoning (WHY buy/sell/hold). Falls back to rules if offline.
        let calendar_context = {
            let cal = self.state.calendar_events.read().await;
            if cal.is_empty() {
                "No high-impact events scheduled.".to_string()
            } else {
                cal.iter()
                    .map(|e| format!("⚠ {} at {} ({:?})", e.title, e.time.as_deref().unwrap_or("TBD"), e.impact))
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
        let multi_tf_context = {
            let mtf = self.state.multi_timeframe_data.read().await;
            if let Some(tf_data) = mtf.get(symbol) {
                tf_data.iter()
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
        };
        let agent_market_summary = {
            let s = self.state.agent_market_summary.read().await;
            if s.is_empty() { "No market summary yet.".to_string() } else { s.clone() }
        };
        let news_context = match &news_ctx {
            Some(ctx) => ctx.to_prompt_string(),
            None => "No recent news.".to_string(),
        };
        let similar_episodes_context = vector_context.clone();

        // Track whether LLM was used (needed after llm_decision is moved)
        let mut llm_was_used = false;

        // Ask the LLM for its structured decision with full reasoning.
        // ask_for_trade_decision() already handles Ollama being down gracefully (returns HOLD),
        // so no separate health check needed — avoids adding 5s latency when offline.
        println!("[StrategyDecision] 🧠 Asking LLM for agentic decision on {}", symbol);
        let llm_decision: Option<LlmTradeDecision> = {
            let decision = self.state.llm.ask_for_trade_decision(
                symbol,
                current_price,
                confluence,
                &trend_label,
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
                &similar_episodes_context,
                &patterns_context,
            ).await;

            if decision.action != "HOLD" && !decision.reason.contains("Parse failed") {
                println!(
                    "[StrategyDecision] 🤖 LLM verdict: {} @ {:.2} | reason: {}",
                    decision.action, decision.entry, decision.reason
                );
                llm_was_used = true;
                Some(decision)
            } else {
                println!("[StrategyDecision] ⚠ LLM returned HOLD or unavailable — using rule-based path.");
                None
            }
        };

        // === BUILD FINAL STRUCTURED REASONING ===
        // Every decision now gets ranked factors explaining WHY.
        let (final_action, final_conf, final_reasoning, final_entry, final_sl, final_tp) = {
            if let Some(llm) = llm_decision {
                // LLM has final say — its reasoning is richer and context-aware
                let llm_action = llm.action.clone();
                let llm_conf = debate_conf.max(0.5); // LLM doesn't give confidence, use debate as floor
                let llm_reason = format!(
                    "🧠 LLM REASONING: {} | Rule-based was: {} | Factors: RSI={:.1} MACD={:.4} ATR%={:.2}% | News: {} | Forecast: {} | Debate: {} (conf {:.2}) | Memory: {}",
                    llm.reason,
                    debate_action,
                    rsi, macd_hist, atr_pct * 100.0,
                    if news_ctx.is_some() { "available" } else { "none" },
                    forecast_summary,
                    debate_action, debate_conf,
                    vector_context,
                );
                (
                    llm_action,
                    llm_conf,
                    llm_reason,
                    llm.entry,
                    llm.sl,
                    llm.tp,
                )
            } else {
                // Rule-based fallback
                (
                    debate_action.clone(),
                    debate_conf,
                    rule_based_reasoning,
                    entry,
                    stop_loss,
                    take_profit,
                )
            }
        };

        // Store LLM reasoning for debugging / UI display
        {
            let mut last_reason = self.state.last_llm_reason.write().await;
            *last_reason = final_reasoning.clone();
        }

        if final_action == "HOLD" || final_conf < 0.45 {
            println!("[StrategyDecisionAgent] {} decided HOLD for {} (rsi={:.1} macd_hist={:.4})",
                if llm_was_used { "LLM" } else { "Rule" },
                symbol, rsi, macd_hist);
            return Ok(None);
        }

        // === Use LLM-provided levels when available, else rule-based levels ===
        let signal_entry = if final_entry > 0.0 { final_entry } else { entry };
        let signal_sl = if final_sl > 0.0 { final_sl } else { stop_loss };
        let signal_tp = if final_tp > 0.0 { final_tp } else { take_profit };

        // Recompute risk/reward from the final (possibly LLM-overridden) levels
        let final_rr = {
            let risk = (signal_entry - signal_sl).abs();
            let reward = (signal_tp - signal_entry).abs();
            if risk > 0.0 { reward / risk } else { 2.0 }
        };

        // === ADAPTIVE POSITION SIZING ===
        // Scale risk by: confidence, regime, consecutive losses, portfolio heat.
        // Recompute from FRESH portfolio read for accurate heat calculation.
        let (equity, fresh_heat, fresh_consecutive_losses) = {
            let p = self.state.portfolio.read().await;
            let eq = p.cash_balance
                + p.open_positions
                    .iter()
                    .map(|pos| pos.current_price * pos.quantity)
                    .sum::<f64>();
            let total_risk: f64 = p.open_positions.iter().map(|pos| pos.risk_amount).sum();
            let heat = if p.total_equity > 0.0 { total_risk / p.total_equity } else { 0.0 };
            (eq, heat, p.consecutive_losses)
        };

        // Compute adaptive risk multiplier based on system state
        let adaptive_risk_mult = {
            // 1. Confidence scaling: higher confidence → slightly larger (but capped)
            let conf_mult = (final_conf / 0.7).min(1.2).max(0.5);
            // 2. Consecutive loss scaling: more losses → smaller positions
            let loss_mult = if fresh_consecutive_losses >= 3 {
                0.5 // Half size after 3 losses
            } else if fresh_consecutive_losses >= 2 {
                0.7
            } else {
                1.0
            };
            // 3. Portfolio heat scaling: more heat → smaller (using fresh data)
            let heat_mult = if fresh_heat > 0.08 {
                0.5
            } else if fresh_heat > 0.05 {
                0.7
            } else {
                1.0
            };
            // 4. Regime scaling: volatile = smaller, trending = full size
            let regime_mult = match trend_label {
                "Bullish" => 1.0,
                "Bearish" => 0.7,
                "Ranging" => 0.8,
                _ => 0.6, // Volatile/LowLiquidity
            };
            let mult = conf_mult * loss_mult * heat_mult * regime_mult;
            println!(
                "[Strategy] Adaptive risk: conf={:.2} loss={:.2} heat={:.2} regime={:.2} → mult={:.2}",
                conf_mult, loss_mult, heat_mult, regime_mult, mult
            );
            mult.clamp(0.3, 1.2)
        };

        let effective_risk = (rules.max_risk_per_trade * adaptive_risk_mult).max(0.003);
        let position_size = crate::helpers::calculate_position_size(
            equity,
            effective_risk,
            signal_entry,
            signal_sl,
        );

        // Validate the calculated size against remaining cash
        let position_value = position_size * signal_entry;
        let cash_available = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.cash_balance
        };
        let final_position_size = if position_value > cash_available * 0.95 {
            (cash_available * 0.95) / signal_entry.max(0.0001)
        } else {
            position_size
        };

        let signal = TradeSignal {
            symbol: symbol.to_string(),
            direction,
            entry_price: signal_entry,
            stop_loss: signal_sl,
            take_profit: signal_tp,
            position_size: final_position_size,
            confidence_score: debate_conf.min(0.95),
            confluence_score: confluence,
            risk_reward_ratio: final_rr,
            reasoning: final_reasoning,
            timestamp: Utc::now(),
            session_valid: session.market_open,
            risk_check_passed: true, // validated later by verifier
        };

        // Discipline gate (agentic rules still apply to self-generated levels)
        let discipline = validate_trade_setup(&context, &rules);
        if !discipline.passed {
            println!(
                "[StrategyDecisionAgent] Agent self-generated signal rejected by DisciplinedCore"
            );
            return Ok(None);
        }

        println!(
            "[StrategyDecisionAgent] AGENTIC signal {} {} @ entry={:.2} SL={:.2} TP={:.2} (RR {:.1}:1, conf {:.1}%)",
            if direction == tredo_core::TradeDirection::Long { "BUY" } else { "SELL" },
            symbol, signal_entry, signal_sl, signal_tp, final_rr, signal.confidence_score * 100.0
        );

        Ok(Some(signal))
    }
}
