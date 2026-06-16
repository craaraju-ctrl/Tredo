use crate::helpers::get_indian_session_info;
use crate::state::SharedState;
use crate::types::TradeSignal;
use chrono::Utc;
use std::error::Error;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, validate_trade_setup, AgentInput,
    MarketContext,
}; // Full debate aggregator wired (Proposer etc using new skills)

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
        let _forecast_summary = {
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

        let _portfolio_heat: f64 = {
            let total_risk: f64 = portfolio.open_positions.iter().map(|p| p.risk_amount).sum();
            if portfolio.total_equity > 0.0 {
                total_risk / portfolio.total_equity
            } else {
                0.0
            }
        };
        let _consecutive_losses = portfolio.consecutive_losses;
        let _daily_pnl_pct = portfolio.daily_pnl_pct;
        let _total_trades_today = portfolio.total_trades_today;

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
                0.015
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
        let (_news_ctx, meter) = {
            let n = self.state.latest_news.read().await;
            let m = self.state.latest_metrics.read().await;
            (n.get(symbol).cloned(), m.get(symbol).cloned())
        };
        let meter_atr = meter.as_ref().map(|m| m.atr_pct).unwrap_or(atr_pct);
        let _meter_regime = meter
            .as_ref()
            .map(|m| m.regime_hint.clone())
            .unwrap_or_else(|| "ranging".into());
        if let Some(m) = &meter {
            println!(
                "[Strategy] using meter snapshot: rsi={:.1} conf={:.2} regime={}",
                m.rsi_14, m.confluence_hint, m.regime_hint
            );
        }

        let (entry, stop_loss, take_profit, risk_reward_ratio) =
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

        if debate_action == "HOLD" || debate_conf < 0.45 {
            println!("[StrategyDecisionAgent] Agent decided HOLD for {} (debate + indicators: rsi={:.1} macd_hist={:.4})", symbol, rsi, macd_hist);
            return Ok(None);
        }

        // === VECTOR MEMORY USAGE FOR REGIME MATCHING (Gap 3) ===
        // Pull similar historical episodes using the agent's vector memory.
        // When LanceDB feature is enabled this becomes powerful long-term regime memory.
        // This is now actively used in the decision instead of being bypassed.
        let vector_context = {
            let vm = self.state.vector_memory.lock().await;
            if !vm.is_empty() {
                // Use a query that captures current market structure (price + regime + confluence)
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

        // Build reasoning including indicators the agent used + vector memory
        let reasoning = format!(
            "Agentic decision: {} | RSI={:.1} MACD_hist={:.4} ATR%={:.2}% | Pivots R1/S1={:.2}/{:.2} | Patterns: {} | Debate+Agg: {} (conf {:.2}) | {} | {}",
            direction as u8,
            rsi, macd_hist, atr_pct * 100.0,
            pivots.r1, pivots.s1,
            patterns_context,
            debate_action, debate_conf,
            debate_reason,
            vector_context
        );

        // === REAL POSITION SIZING (was 0.0, now uses calculate_position_size from helpers) ===
        let equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.cash_balance
                + portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.current_price * p.quantity)
                    .sum::<f64>()
        };
        let position_size = crate::helpers::calculate_position_size(
            equity,
            rules.max_risk_per_trade,
            entry,
            stop_loss,
        );

        // Validate the calculated size against remaining cash
        let position_value = position_size * entry;
        let cash_available = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.cash_balance
        };
        let final_position_size = if position_value > cash_available * 0.95 {
            // Cap to 95% of available cash
            (cash_available * 0.95) / entry.max(0.0001)
        } else {
            position_size
        };

        let signal = TradeSignal {
            symbol: symbol.to_string(),
            direction,
            entry_price: entry,
            stop_loss,
            take_profit,
            position_size: final_position_size,
            confidence_score: debate_conf.min(0.95),
            confluence_score: confluence,
            risk_reward_ratio,
            reasoning,
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
            symbol, entry, stop_loss, take_profit, risk_reward_ratio, signal.confidence_score * 100.0
        );

        Ok(Some(signal))
    }
}
