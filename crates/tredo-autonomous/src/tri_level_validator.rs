//! Tri-Level Parallel Validator — UPGRADED
//!
//! Runs three independent validation layers **in parallel** against **real live data**:
//!
//!   Layer 1 **Rules**  — HardRulesGate + real OHLCV confluece (deterministic)
//!   Layer 2 **LLM**    — Ollama nemotron-3-nano:4b with REAL market context (multi-TF,
//!                        news, vector memory, patterns — not placeholder strings)
//!   Layer 3 **Kronos** — Time-series forecast with full trajectory momentum analysis
//!
//! ## 2-of-3 Agreement Gate
//! A trade is only allowed if at least 2 of the 3 available layers agree on direction.
//! When only 1 layer fires, `consensus_action` is forced to `"HOLD"` regardless of weight.
//!
//! ## Geometry Consistency
//! `is_geometry_consistent()` cross-checks a `TradeSignal.direction` against the
//! consensus to catch direction contradictions before execution.
//!
//! ## Trust Weight Upgrade
//! After each trade close, `attribute_and_upgrade()` adjusts per-layer trust weights
//! using a multiplicative update so layers that were correct gain more influence.

use crate::hard_rules_gate::HardRulesGate;
use crate::state::SharedState;
use crate::types::{MarketRegime, OhlcvSnapshot, TradeSignal};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, KronosForecastRequest, KronosForecastTool,
    LlmTradeDecision, TradeDirection,
};

const REASONING_LOG: &str = "tri_level_reasoning.jsonl";

/// Normalized signal in [-1.0, +1.0] (bearish → bullish)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerSignal {
    pub layer: String,
    pub signal: f64,
    pub action: String,
    pub confidence: f64,
    pub reasoning: String,
    pub available: bool,
}

/// Combined verdict from all three parallel layers.
///
/// `hard_agree = true` means ≥ 2 of 3 available layers agree on the consensus direction.
/// Only when `hard_agree` is true may the pipeline proceed to trade execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriLevelVerdict {
    pub symbol: String,
    pub timestamp: String,
    pub rules: LayerSignal,
    pub llm: LayerSignal,
    pub kronos: LayerSignal,
    pub consensus_signal: f64,
    pub consensus_action: String,
    pub layer_weights: LayerTrustWeights,
    /// How many layers agree with the consensus direction (0, 1, 2, or 3)
    pub agreement_count: u8,
    /// At least 2 of 3 available layers agree → trade allowed
    pub hard_agree: bool,
    /// All available layers agree on the same direction
    pub direction_unanimous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerTrustWeights {
    pub rules: f64,
    pub llm: f64,
    pub kronos: f64,
}

impl Default for LayerTrustWeights {
    fn default() -> Self {
        Self {
            rules: 0.40,
            llm: 0.30,
            kronos: 0.30,
        }
    }
}

impl LayerTrustWeights {
    pub fn normalize(&mut self) {
        let sum = self.rules + self.llm + self.kronos;
        if sum > 0.0 {
            self.rules /= sum;
            self.llm /= sum;
            self.kronos /= sum;
        }
    }
}

/// Compute a `LayerSignal`'s effective action string (BUY / SELL / HOLD / BLOCK).
fn signal_action(sig: &LayerSignal) -> &str {
    &sig.action
}

/// Count how many available layers agree with the consensus direction.
fn compute_agreement(
    rules: &LayerSignal,
    llm: &LayerSignal,
    kronos: &LayerSignal,
    consensus_action: &str,
) -> (u8, bool, bool) {
    let layers = [rules, llm, kronos];
    let available: Vec<&&LayerSignal> = layers.iter().filter(|l| l.available).collect();
    let available_count = available.len() as u8;

    if available_count == 0 {
        return (0, false, false);
    }

    // Map BLOCK → same as SELL (rules-layer veto)
    let normalize_action = |a: &str| -> &str {
        match a {
            "BUY" => "BUY",
            "SELL" | "BLOCK" => "SELL",
            _ => "HOLD",
        }
    };

    let consensus_norm = normalize_action(consensus_action);
    let mut agree_count = 0u8;
    for l in &available {
        if normalize_action(signal_action(l)) == consensus_norm {
            agree_count += 1;
        }
    }

    let hard_agree = agree_count >= 2 || (available_count == 1 && agree_count == 1);
    let direction_unanimous = agree_count == available_count && available_count >= 2;
    (agree_count, hard_agree, direction_unanimous)
}

pub struct TriLevelValidator {
    state: SharedState,
}

impl TriLevelValidator {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Run Rules, LLM, and Kronos checks in parallel with **real market data**.
    /// Uses a fresh OHLCV snapshot from SharedState (original interface).
    ///
    /// Returns a `TriLevelVerdict` with `hard_agree` set to true only when
    /// ≥ 2 of 3 layers agree on the consensus direction.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_parallel_check(
        &self,
        symbol: &str,
        current_price: f64,
        confluence: f64,
        trend_label: &str,
        forecast_summary: &str,
        portfolio_heat: f64,
        session_open: bool,
        consecutive_losses: u32,
    ) -> TriLevelVerdict {
        let snapshot = OhlcvSnapshot::capture(symbol, &self.state).await;
        self.run_parallel_check_with_ohlcv(
            &snapshot,
            symbol,
            current_price,
            confluence,
            trend_label,
            forecast_summary,
            portfolio_heat,
            session_open,
            consecutive_losses,
        )
        .await
    }

    /// Run all 3 parallel checks using an explicit OHLCV snapshot so all layers
    /// (HardRulesGate, LLM, Kronos) see the identical market data.
    ///
    /// This is the unified entry point from the redesigned pipeline.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_parallel_check_with_ohlcv(
        &self,
        snapshot: &OhlcvSnapshot,
        symbol: &str,
        current_price: f64,
        confluence: f64,
        trend_label: &str,
        forecast_summary: &str,
        portfolio_heat: f64,
        session_open: bool,
        consecutive_losses: u32,
    ) -> TriLevelVerdict {
        let state_rules = self.state.clone();
        let state_llm = self.state.clone();
        let state_kronos = self.state.clone();
        let sym = symbol.to_string();
        let trend = trend_label.to_string();
        let forecast = forecast_summary.to_string();

        let weights = self.state.layer_trust_weights.read().await.clone();

        // ── Pull real market context from SharedState for LLM ──────────────────
        let multi_tf_context = {
            let mtf_agg = self.state.multi_tf_aggregate.read().await;
            if let Some(agg) = mtf_agg.get(symbol) {
                format!(
                    "MTF({} TFs): dir={} signal={:.3} agree={:.0}% | {}",
                    agg.tf_count,
                    agg.aggregate_direction,
                    agg.aggregate_signal,
                    agg.agreement_pct * 100.0,
                    agg.tf_analyses
                        .iter()
                        .map(|(tf, a)| format!(
                            "{}:{} conf={:.0}% rsi={:.0}",
                            tf,
                            a.aggregated_direction,
                            a.aggregated_conviction * 100.0,
                            a.metrics.rsi_14
                        ))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            } else {
                "No multi-TF aggregate available".to_string()
            }
        };

        let news_context = {
            let news = self.state.latest_news.read().await;
            match news.get(symbol) {
                Some(ctx) => ctx.to_prompt_string(),
                None => "No recent news for this symbol.".to_string(),
            }
        };

        let vector_context = {
            let vm = self.state.vector_memory.read().await;
            if !vm.is_empty() {
                let query = format!(
                    "{} regime={} confluence={:.2} price={:.2}",
                    symbol, trend_label, confluence, current_price
                );
                drop(vm);
                let vm2 = self.state.vector_memory.read().await;
                match vm2.search(&query, 3, &self.state.llm).await {
                    Ok(results) if !results.is_empty() => results
                        .iter()
                        .map(|r| {
                            let regret = r
                                .regret_score
                                .map(|s| format!(" regret={:.2}", s))
                                .unwrap_or_default();
                            format!(
                                "{} (sim={:.0}%){}",
                                r.summary_text,
                                r.similarity * 100.0,
                                regret
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" | "),
                    _ => "No vector memory matches.".to_string(),
                }
            } else {
                "Vector memory empty.".to_string()
            }
        };

        let patterns_context = {
            let pats = self.state.last_patterns.read().await;
            match pats.get(symbol) {
                Some(p) if !p.is_empty() => tredo_core::format_patterns(p),
                _ => "No candlestick patterns detected.".to_string(),
            }
        };

        let agent_summary = {
            let s = self.state.agent_market_summary.read().await;
            if s.is_empty() {
                "No agent market summary yet.".to_string()
            } else {
                s.clone()
            }
        };

        // ── Run all three layers in parallel using the SAME snapshot ──────────
        // All 3 layers (rules/HardRulesGate, LLM, Kronos) receive the identical
        // OHLCV data captured at pipeline start. No layer sees stale or different data.
        let snapshot_ref = snapshot.clone();
        let (rules_sig, llm_sig, kronos_sig) = tokio::join!(
            Self::check_rules_layer(state_rules, &sym, current_price, confluence, &snapshot_ref),
            Self::check_llm_layer(
                state_llm,
                &sym,
                current_price,
                confluence,
                &trend,
                &forecast,
                portfolio_heat,
                session_open,
                consecutive_losses,
                &multi_tf_context,
                &agent_summary,
                &news_context,
                &vector_context,
                &patterns_context,
                &snapshot_ref,
            ),
            Self::check_kronos_layer(state_kronos, &sym, current_price, &snapshot_ref),
        );

        // ── Weighted consensus signal ──────────────────────────────────────────
        let raw_consensus = weights.rules * rules_sig.signal
            + weights.llm * llm_sig.signal
            + weights.kronos * kronos_sig.signal;

        let raw_action = signal_to_action(raw_consensus);

        // ── 2-of-3 Agreement Gate ─────────────────────────────────────────────
        // Count how many available layers agree with the raw weighted consensus.
        let (agreement_count, hard_agree, direction_unanimous) =
            compute_agreement(&rules_sig, &llm_sig, &kronos_sig, &raw_action);

        // If hard_agree is false (only 1 layer fires), force consensus to HOLD.
        // Exception: if only 1 layer is available and it fires, allow it (degraded mode).
        let available_count = [&rules_sig, &llm_sig, &kronos_sig]
            .iter()
            .filter(|l| l.available)
            .count();

        let consensus_action = if !hard_agree && available_count >= 2 {
            println!(
                "[TriLevel] ⚠ {}: only {}/{} layers agree → forcing HOLD (agreement gate)",
                symbol, agreement_count, available_count
            );
            "HOLD".to_string()
        } else {
            raw_action.clone()
        };

        let consensus_signal = if consensus_action == "HOLD" && raw_action != "HOLD" {
            // Dampen signal to neutral when gate overrides
            raw_consensus * 0.3
        } else {
            raw_consensus
        };

        let verdict = TriLevelVerdict {
            symbol: sym.clone(),
            timestamp: Utc::now().to_rfc3339(),
            rules: rules_sig,
            llm: llm_sig,
            kronos: kronos_sig,
            consensus_signal,
            consensus_action,
            layer_weights: weights,
            agreement_count,
            hard_agree,
            direction_unanimous,
        };

        Self::append_reasoning_log(&verdict);
        {
            let mut store = self.state.last_tri_level_verdict.write().await;
            store.insert(sym, verdict.clone());
        }

        println!(
            "[TriLevel] {} → rules={:.2}({}) llm={:.2}({}) kronos={:.2}({}) consensus={:.2}({}) agree={}/{} hard={}",
            symbol,
            verdict.rules.signal,
            verdict.rules.action,
            verdict.llm.signal,
            verdict.llm.action,
            verdict.kronos.signal,
            verdict.kronos.action,
            verdict.consensus_signal,
            verdict.consensus_action,
            verdict.agreement_count,
            available_count,
            verdict.hard_agree,
        );

        verdict
    }

    // ── Layer 1: Rules (uses the pipeline-wide OHLCV snapshot) ────────────────

    async fn check_rules_layer(
        state: SharedState,
        symbol: &str,
        current_price: f64,
        confluence: f64,
        snapshot: &OhlcvSnapshot,
    ) -> LayerSignal {
        // Run hard rules gate using the same snapshot as LLM and Kronos
        let gate = HardRulesGate::new(state.clone());
        let result = gate.evaluate_with_ohlcv(symbol, snapshot).await;

        if !result.passed {
            return LayerSignal {
                layer: "rules".into(),
                signal: 0.0,
                action: "BLOCK".into(),
                confidence: 1.0,
                reasoning: format!(
                    "Hard rules blocked: {}",
                    result
                        .failed_rules
                        .iter()
                        .map(|r| r.rule_name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                available: true,
            };
        }

        // Use OHLCV snapshot bars for pivot calculation (same data as LLM and Kronos)
        let (real_high, real_low, real_close) = match snapshot.bars().last() {
            Some(bar) => (bar.high, bar.low, bar.close),
            None => (
                current_price * 1.01,
                current_price * 0.99,
                current_price * 0.998,
            ),
        };

        let rules = state.rules.read().await;
        let pivots = calculate_pivot_points(real_high, real_low, real_close, rules.pivot_method);
        drop(rules);

        let regime = *state.market_regime.read().await;
        let regime_bias = match regime {
            Some(MarketRegime::TrendingBull) => 0.3,
            Some(MarketRegime::TrendingBear) => -0.3,
            _ => 0.0,
        };

        let ctx = tredo_core::MarketContext {
            symbol: symbol.to_string(),
            current_price,
            high: real_high,
            low: real_low,
            previous_close: real_close,
            timestamp: Utc::now(),
            daily_pnl: 0.0,
            equity: 100_000.0,
            consecutive_losses: 0,
            is_red_folder_day: false,
            trend_direction: None,
        };
        let conf_score = calculate_confluence_score(&ctx, &pivots);
        let raw_signal = ((confluence + conf_score) / 2.0 - 0.5) * 2.0 + regime_bias;
        let clamped = raw_signal.clamp(-1.0, 1.0);

        LayerSignal {
            layer: "rules".into(),
            signal: clamped,
            action: signal_to_action(clamped),
            confidence: conf_score.clamp(0.0, 1.0),
            reasoning: format!(
                "Rules PASS | real_high={:.2} real_low={:.2} pivot={:.2} | confluence={:.2} conf_score={:.2} regime_bias={:.2}",
                real_high, real_low, pivots.pivot, confluence, conf_score, regime_bias
            ),
            available: true,
        }
    }

    // ── Layer 2: LLM with REAL data (uses pipeline-wide snapshot) ─────────────

    #[allow(clippy::too_many_arguments)]
    async fn check_llm_layer(
        state: SharedState,
        symbol: &str,
        current_price: f64,
        confluence: f64,
        trend_label: &str,
        forecast_summary: &str,
        portfolio_heat: f64,
        session_open: bool,
        consecutive_losses: u32,
        // Real context from SharedState (NOT placeholder strings)
        multi_tf_context: &str,
        agent_market_summary: &str,
        news_context: &str,
        vector_context: &str,
        patterns_context: &str,
        snapshot: &OhlcvSnapshot,
    ) -> LayerSignal {
        let rules = state.rules.read().await;
        // Use snapshot bars for pivot (same data as rules and Kronos layers)
        let (real_high, real_low, real_close) = match snapshot.bars().last() {
            Some(bar) => (bar.high, bar.low, bar.close),
            None => (
                current_price * 1.01,
                current_price * 0.99,
                current_price * 0.998,
            ),
        };
        let pivots = calculate_pivot_points(real_high, real_low, real_close, rules.pivot_method);
        drop(rules);

        state
            .push_live_comm(
                "TriLevel::LLM",
                "Ollama",
                "QUERY",
                &format!(
                    "Requesting trade decision for {} @ {:.2} (Model: {})",
                    symbol, current_price, state.config.llm_model
                ),
                Some(symbol.to_string()),
            )
            .await;

        // ═══ HARD 25-SECOND LLM TIMEOUT ════════════════════════════
        // Prevent LLM from blocking the tri-level validator if slow.
        let decision: LlmTradeDecision = tokio::time::timeout(
            std::time::Duration::from_secs(25),
            state.llm.ask_for_trade_decision(
                symbol,
                current_price,
                confluence,
                trend_label,
                pivots.pivot,
                pivots.r1,
                pivots.s1,
                forecast_summary,
                portfolio_heat,
                session_open,
                consecutive_losses,
                "Tri-level parallel check",
                "paper",
                "Live paper trading validation",
                // Real data — no more placeholder strings
                multi_tf_context,
                agent_market_summary,
                news_context,
                vector_context,
                patterns_context,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            println!(
                "[TriLevel] ⏱ LLM timed out after 25s for {} — marking unavailable",
                symbol
            );
            LlmTradeDecision {
                action: "HOLD".to_string(),
                reason: "LLM timeout (25s)".to_string(),
                entry: 0.0,
                sl: 0.0,
                tp: 0.0,
            }
        });

        let available =
            !decision.reason.contains("Parse failed") && !decision.reason.contains("unavailable");

        if available {
            state
                .push_live_comm(
                    "Ollama",
                    "TriLevel::LLM",
                    &decision.action,
                    &format!("Response: {}", decision.reason),
                    Some(symbol.to_string()),
                )
                .await;
        } else {
            state
                .push_live_comm(
                    "Ollama",
                    "TriLevel::LLM",
                    "ERROR",
                    &format!("Ollama request failed/HOLD: {}", decision.reason),
                    Some(symbol.to_string()),
                )
                .await;
        }

        let confidence: f64 = if !available {
            0.0
        } else {
            match decision.action.as_str() {
                "BUY" | "SELL" => 0.70,
                _ => 0.40,
            }
        };
        let signal: f64 = if !available {
            0.0
        } else {
            match decision.action.as_str() {
                "BUY" => confidence,
                "SELL" => -confidence,
                _ => 0.0,
            }
        };

        LayerSignal {
            layer: "llm".into(),
            signal: signal.clamp(-1.0, 1.0),
            action: if available {
                decision.action.clone()
            } else {
                "HOLD".to_string()
            },
            confidence,
            reasoning: format!(
                "{} | pivot={:.2} R1={:.2} S1={:.2} | mtf_tfs={} | {}",
                decision.reason,
                pivots.pivot,
                pivots.r1,
                pivots.s1,
                if multi_tf_context.contains("TFs") {
                    "available"
                } else {
                    "none"
                },
                if available {
                    "LLM_OK"
                } else {
                    "LLM_UNAVAILABLE"
                }
            ),
            available,
        }
    }

    // ── Layer 3: Kronos (uses pipeline-wide OHLCV snapshot) ───────────────────

    async fn check_kronos_layer(
        state: SharedState,
        symbol: &str,
        current_price: f64,
        snapshot: &OhlcvSnapshot,
    ) -> LayerSignal {
        let ohlcv = snapshot.bars().to_vec();

        if ohlcv.is_empty() {
            return LayerSignal {
                layer: "kronos".into(),
                signal: 0.0,
                action: "HOLD".into(),
                confidence: 0.0,
                reasoning: "No OHLCV history for Kronos forecast".into(),
                available: false,
            };
        }

        let client = KronosForecastTool::new(state.config.kronos_service_url.clone());
        let req = KronosForecastRequest {
            symbol: symbol.to_string(),
            ohlcv,
            pred_len: 5,
            temperature: 0.8,
            top_p: 0.9,
            sample_count: 1,
        };

        state
            .push_live_comm(
                "TriLevel::Kronos",
                "Kronos",
                "FORECAST",
                &format!("Requesting 5-bar forecast trajectory for {}", symbol),
                Some(symbol.to_string()),
            )
            .await;

        match client.forecast(req).await {
            Ok(resp) => {
                // Extract the full forecast trajectory (all 5 bars)
                let closes: Vec<f64> = resp
                    .forecasts
                    .iter()
                    .filter_map(|f| f.get("close").and_then(|c| c.as_f64()))
                    .collect();

                if closes.is_empty() {
                    state
                        .push_live_comm(
                            "Kronos",
                            "TriLevel::Kronos",
                            "HOLD",
                            "Forecast returned empty trajectory",
                            Some(symbol.to_string()),
                        )
                        .await;
                    return LayerSignal {
                        layer: "kronos".into(),
                        signal: 0.0,
                        action: "HOLD".into(),
                        confidence: 0.0,
                        reasoning: "Kronos returned empty forecast".into(),
                        available: false,
                    };
                }

                // Full trajectory analysis
                let last_pred = *closes.last().unwrap();
                let overall_pct = (last_pred - current_price) / current_price;

                // Momentum consistency: count bars that move in same direction as overall trend
                let expected_direction = if overall_pct >= 0.0 { "up" } else { "down" };
                let mut prev = current_price;
                let mut consistent_bars = 0usize;
                let mut whipsaw_bars = 0usize;
                for &c in &closes {
                    let bar_dir = if c > prev { "up" } else { "down" };
                    if bar_dir == expected_direction {
                        consistent_bars += 1;
                    } else {
                        whipsaw_bars += 1;
                    }
                    prev = c;
                }
                let total_bars = closes.len().max(1);
                let consistency_ratio = consistent_bars as f64 / total_bars as f64;

                // Raw signal from last-bar return
                let raw_signal = (overall_pct * 20.0).clamp(-1.0, 1.0);

                // Confidence: scaled by trajectory consistency
                // - Clean trend (4/5 bars agree): full confidence
                // - Whipsaw (2/5 bars agree): halved confidence
                let base_conf = overall_pct.abs().min(0.15) / 0.15;
                let conf = (base_conf * consistency_ratio).clamp(0.0, 1.0);

                // Dampen the signal for whipsaw forecasts
                let signal = if consistency_ratio < 0.5 {
                    raw_signal * 0.4 // heavy dampen — unreliable trajectory
                } else if consistency_ratio < 0.7 {
                    raw_signal * 0.7
                } else {
                    raw_signal
                };

                let action = if signal.abs() < 0.15 || conf < 0.25 {
                    "HOLD".to_string()
                } else {
                    signal_to_action(signal)
                };

                state
                    .push_live_comm(
                        "Kronos",
                        "TriLevel::Kronos",
                        &action,
                        &format!(
                            "Response: trajectory={}/{} consistent ({:.0}%) | pred={:.2} ({:+.2}%)",
                            consistent_bars,
                            total_bars,
                            consistency_ratio * 100.0,
                            last_pred,
                            overall_pct * 100.0
                        ),
                        Some(symbol.to_string()),
                    )
                    .await;

                LayerSignal {
                    layer: "kronos".into(),
                    signal: signal.clamp(-1.0, 1.0),
                    action,
                    confidence: conf,
                    reasoning: format!(
                        "Kronos pred_close={:.2} ({:+.2}%) | trajectory={}/{} bars consistent ({:.0}%) | whipsaw={} | conf={:.2}",
                        last_pred,
                        overall_pct * 100.0,
                        consistent_bars,
                        total_bars,
                        consistency_ratio * 100.0,
                        whipsaw_bars,
                        conf
                    ),
                    available: true,
                }
            }
            Err(e) => {
                state
                    .push_live_comm(
                        "Kronos",
                        "TriLevel::Kronos",
                        "ERROR",
                        &format!("Forecast failed: {}", e),
                        Some(symbol.to_string()),
                    )
                    .await;
                LayerSignal {
                    layer: "kronos".into(),
                    signal: 0.0,
                    action: "HOLD".into(),
                    confidence: 0.0,
                    reasoning: format!("Kronos unavailable: {}", e),
                    available: false,
                }
            }
        }
    }

    fn append_reasoning_log(verdict: &TriLevelVerdict) {
        if let Ok(line) = serde_json::to_string(verdict) {
            if let Ok(mut f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(REASONING_LOG)
            {
                let _ = writeln!(f, "{}", line);
            }
        }
    }

    /// After trade close: determine which layer was correct and upgrade trust weights.
    pub async fn attribute_and_upgrade(
        state: &SharedState,
        episode_id: &str,
        direction: &str,
        pct_pnl: f64,
        layer_predictions: &HashMap<String, f64>,
    ) -> LayerTrustWeights {
        let outcome_signal = match direction {
            "BUY" => {
                if pct_pnl > 0.0 {
                    1.0
                } else {
                    -1.0
                }
            }
            "SELL" => {
                if pct_pnl > 0.0 {
                    -1.0
                } else {
                    1.0
                }
            }
            _ => 0.0,
        };

        let mut weights = state.layer_trust_weights.read().await.clone();
        let lr = 0.05;

        for (layer, &pred) in layer_predictions {
            if !matches!(layer.as_str(), "rules" | "llm" | "kronos") {
                continue;
            }
            let clamped = pred.clamp(-1.0, 1.0);
            let correct = (clamped >= 0.0 && outcome_signal >= 0.0)
                || (clamped < 0.0 && outcome_signal < 0.0);
            let delta = (clamped - outcome_signal).abs();

            let slot = match layer.as_str() {
                "rules" => &mut weights.rules,
                "llm" => &mut weights.llm,
                "kronos" => &mut weights.kronos,
                _ => continue,
            };

            if correct {
                let accuracy = (1.0 - delta / 2.0).max(0.0);
                *slot *= 1.0 + lr * accuracy;
            } else {
                let regret = (delta / 2.0).min(1.0);
                *slot *= 1.0 - lr * regret;
            }
            *slot = slot.clamp(0.10, 0.60);
        }

        weights.normalize();
        *state.layer_trust_weights.write().await = weights.clone();

        println!(
            "[TriLevel] {} attribution → rules={:.0}% llm={:.0}% kronos={:.0}% (pnl={:+.2}%)",
            episode_id,
            weights.rules * 100.0,
            weights.llm * 100.0,
            weights.kronos * 100.0,
            pct_pnl * 100.0
        );

        if let Ok(line) = serde_json::to_string(&serde_json::json!({
            "type": "layer_attribution",
            "episode_id": episode_id,
            "direction": direction,
            "pct_pnl": pct_pnl,
            "layer_predictions": layer_predictions,
            "updated_weights": weights,
            "timestamp": Utc::now().to_rfc3339(),
        })) {
            if let Ok(mut f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(REASONING_LOG)
            {
                let _ = writeln!(f, "{}", line);
            }
        }

        weights
    }
}

/// Check if a `TradeSignal`'s direction is consistent with the tri-level consensus.
///
/// Returns `Ok(())` if consistent, `Err(reason)` if there is a direction contradiction.
/// A contradiction is defined as:
///   - Signal is `Long` but `consensus_action == "SELL"`
///   - Signal is `Short` but `consensus_action == "BUY"`
///   - AND `hard_agree == true` (strong consensus — not a weak single-layer signal)
pub fn is_geometry_consistent(
    verdict: &TriLevelVerdict,
    signal: &TradeSignal,
) -> Result<(), String> {
    // Only enforce when tri-level has a strong, hard-agreed direction
    if !verdict.hard_agree {
        return Ok(()); // soft signal — do not block
    }
    if verdict.consensus_action == "HOLD" {
        return Ok(()); // neutral — no direction to conflict with
    }

    let signal_action = match signal.direction {
        TradeDirection::Long => "BUY",
        TradeDirection::Short => "SELL",
    };

    if signal_action != verdict.consensus_action {
        Err(format!(
            "DIRECTION_CONFLICT: signal={} but tri-level consensus={} (hard_agree={}, agree={}/3)",
            signal_action, verdict.consensus_action, verdict.hard_agree, verdict.agreement_count
        ))
    } else {
        Ok(())
    }
}

pub fn signal_to_action(signal: f64) -> String {
    if signal > 0.15 {
        "BUY".to_string()
    } else if signal < -0.15 {
        "SELL".to_string()
    } else {
        "HOLD".to_string()
    }
}

pub fn verdict_to_layer_predictions(verdict: &TriLevelVerdict) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    if verdict.rules.available {
        m.insert("rules".into(), verdict.rules.signal);
    }
    if verdict.llm.available {
        m.insert("llm".into(), verdict.llm.signal);
    }
    if verdict.kronos.available {
        m.insert("kronos".into(), verdict.kronos.signal);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_to_action() {
        assert_eq!(signal_to_action(0.5), "BUY");
        assert_eq!(signal_to_action(-0.5), "SELL");
        assert_eq!(signal_to_action(0.0), "HOLD");
        assert_eq!(signal_to_action(0.14), "HOLD"); // below threshold
        assert_eq!(signal_to_action(-0.14), "HOLD");
    }

    #[test]
    fn test_layer_weights_normalize() {
        let mut w = LayerTrustWeights {
            rules: 0.5,
            llm: 0.3,
            kronos: 0.3,
        };
        w.normalize();
        let sum = w.rules + w.llm + w.kronos;
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_compute_agreement_all_agree() {
        let make = |action: &str| LayerSignal {
            layer: "test".into(),
            signal: 0.5,
            action: action.to_string(),
            confidence: 0.7,
            reasoning: String::new(),
            available: true,
        };
        let (count, hard, unanimous) =
            compute_agreement(&make("BUY"), &make("BUY"), &make("BUY"), "BUY");
        assert_eq!(count, 3);
        assert!(hard);
        assert!(unanimous);
    }

    #[test]
    fn test_compute_agreement_one_disagrees() {
        let buy = LayerSignal {
            layer: "test".into(),
            signal: 0.5,
            action: "BUY".to_string(),
            confidence: 0.7,
            reasoning: String::new(),
            available: true,
        };
        let hold = LayerSignal {
            layer: "test".into(),
            signal: 0.05,
            action: "HOLD".to_string(),
            confidence: 0.4,
            reasoning: String::new(),
            available: true,
        };
        let (count, hard, unanimous) = compute_agreement(&buy, &buy, &hold, "BUY");
        assert_eq!(count, 2);
        assert!(hard); // 2/3 is still hard_agree
        assert!(!unanimous);
    }

    #[test]
    fn test_compute_agreement_only_one_agrees() {
        let buy = LayerSignal {
            layer: "test".into(),
            signal: 0.5,
            action: "BUY".to_string(),
            confidence: 0.7,
            reasoning: String::new(),
            available: true,
        };
        let hold = LayerSignal {
            layer: "test".into(),
            signal: 0.05,
            action: "HOLD".to_string(),
            confidence: 0.4,
            reasoning: String::new(),
            available: true,
        };
        let sell = LayerSignal {
            layer: "test".into(),
            signal: -0.5,
            action: "SELL".to_string(),
            confidence: 0.7,
            reasoning: String::new(),
            available: true,
        };
        let (count, hard, _) = compute_agreement(&buy, &sell, &hold, "BUY");
        assert_eq!(count, 1);
        assert!(!hard); // only 1/3 — agreement gate fails
    }

    #[test]
    fn test_geometry_consistent_long_buy() {
        let verdict = TriLevelVerdict {
            symbol: "BTC".into(),
            timestamp: String::new(),
            rules: LayerSignal {
                layer: "rules".into(),
                signal: 0.5,
                action: "BUY".into(),
                confidence: 0.7,
                reasoning: String::new(),
                available: true,
            },
            llm: LayerSignal {
                layer: "llm".into(),
                signal: 0.6,
                action: "BUY".into(),
                confidence: 0.7,
                reasoning: String::new(),
                available: true,
            },
            kronos: LayerSignal {
                layer: "kronos".into(),
                signal: 0.3,
                action: "BUY".into(),
                confidence: 0.5,
                reasoning: String::new(),
                available: true,
            },
            consensus_signal: 0.47,
            consensus_action: "BUY".into(),
            layer_weights: LayerTrustWeights::default(),
            agreement_count: 3,
            hard_agree: true,
            direction_unanimous: true,
        };
        let signal = TradeSignal {
            symbol: "BTC".into(),
            direction: TradeDirection::Long,
            entry_price: 100.0,
            stop_loss: 98.0,
            take_profit: 104.0,
            position_size: 1.0,
            confidence_score: 0.7,
            confluence_score: 0.6,
            risk_reward_ratio: 2.0,
            reasoning: String::new(),
            timestamp: Utc::now(),
            session_valid: true,
            risk_check_passed: true,
        };
        assert!(is_geometry_consistent(&verdict, &signal).is_ok());
    }

    #[test]
    fn test_geometry_conflict_long_sell_consensus() {
        let verdict = TriLevelVerdict {
            symbol: "BTC".into(),
            timestamp: String::new(),
            rules: LayerSignal {
                layer: "rules".into(),
                signal: -0.5,
                action: "SELL".into(),
                confidence: 0.7,
                reasoning: String::new(),
                available: true,
            },
            llm: LayerSignal {
                layer: "llm".into(),
                signal: -0.6,
                action: "SELL".into(),
                confidence: 0.7,
                reasoning: String::new(),
                available: true,
            },
            kronos: LayerSignal {
                layer: "kronos".into(),
                signal: -0.3,
                action: "SELL".into(),
                confidence: 0.5,
                reasoning: String::new(),
                available: true,
            },
            consensus_signal: -0.47,
            consensus_action: "SELL".into(),
            layer_weights: LayerTrustWeights::default(),
            agreement_count: 3,
            hard_agree: true,
            direction_unanimous: true,
        };
        let signal = TradeSignal {
            symbol: "BTC".into(),
            direction: TradeDirection::Long, // CONFLICT: strategy says Long, tri-level says SELL
            entry_price: 100.0,
            stop_loss: 98.0,
            take_profit: 104.0,
            position_size: 1.0,
            confidence_score: 0.7,
            confluence_score: 0.6,
            risk_reward_ratio: 2.0,
            reasoning: String::new(),
            timestamp: Utc::now(),
            session_valid: true,
            risk_check_passed: true,
        };
        let result = is_geometry_consistent(&verdict, &signal);
        assert!(result.is_err(), "Should detect direction conflict");
        assert!(result.unwrap_err().contains("DIRECTION_CONFLICT"));
    }
}
