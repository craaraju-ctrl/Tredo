// ═══════════════════════════════════════════════════════════════════════════════
// High-Level Debate Layer — Multi-round adversarial decision system
//
// Architecture (mirrors real hedge fund decision process):
//
//   Identifier → Verifier → ╔══ DEBATE LAYER ══╗ → Executer
//                            ║ Round 1: Propose  ║
//                            ║ Round 2: Adversarial ║
//                            ║ Round 3: Synthesis  ║
//                            ║ Judge: Adjudicate   ║
//                            ╚════════════════════╝
//
// Agents:
//   - BullTeam: Builds the strongest possible case for entry
//   - BearTeam: Builds the strongest possible case against entry
//   - Synthesizer: Merges both cases into a balanced verdict
//   - Judge: Independent adjudicator — evaluates debate quality ONLY
//
// KEY: The Judge does NOT re-run risk/regime/confluence checks.
// Those are enforced by the HardRulesGate (Layer 1).
// The Judge only evaluates whether the debate evidence supports action.
//
// No LLM dependency — all intelligence is evidence-based and regime-adaptive.
// ═══════════════════════════════════════════════════════════════════════════════

use crate::debate::EvidenceBuilder;
use crate::state::SharedState;
use crate::types::{MarketRegime, TradeSignal};

// ═══════════════════════════════════════════════════════════════════════════════
// Debate Layer Types
// ═══════════════════════════════════════════════════════════════════════════════

/// A single round's output from a debate participant.
/// NOTE: Debate agents are ADVISORY only. They provide evidence + confidence.
/// Only the Judge (Layer 4) has decision-making power (BUY/HOLD/SELL).
/// Debate agents return BUY/HOLD/SELL recommendations, never BLOCK.
#[derive(Clone, Debug)]
pub struct DebateRoundOutput {
    pub participant: String,
    pub action: String,          // "BUY", "SELL", "HOLD" (advisory recommendations)
    pub confidence: f64,
    pub evidence: EvidenceBuilder,
    pub reasoning: String,
    pub key_challenges: Vec<String>, // what this participant would challenge
}

/// The final verdict from the debate layer
#[derive(Clone, Debug)]
pub struct DebateVerdict {
    pub action: String,         // "BUY", "SELL", "HOLD"
    pub confidence: f64,
    pub reasoning: String,
    pub evidence_summary: String,
    pub rounds_played: u32,
    pub judge_veto: bool,
    pub appeal_used: bool,
}

/// Pipeline context passed into the debate layer
#[derive(Clone, Debug)]
pub struct DebateContext {
    pub symbol: String,
    pub price: f64,
    pub regime: MarketRegime,
    pub regime_label: String,
    pub confluence: f64,
    pub rsi: f64,
    pub macd_hist: f64,
    pub atr_pct: f64,
    pub portfolio_heat: f64,
    pub consecutive_losses: u32,
    pub news_available: bool,
    pub patterns_count: usize,
    pub vector_memory_matches: usize,
    pub aggregated_signal: Option<tredo_core::AggregatedSignal>,
    pub skill_votes_bullish: usize,
    pub skill_votes_bearish: usize,
    pub skill_votes_neutral: usize,
    // === NEW INDICATORS: 5 additional independent signals ===
    pub obv_direction: f64,   // OBV trend: >0 bullish volume, <0 bearish volume
    pub adx: f64,             // ADX trend strength (0-100), >25 = trending
    pub plus_di: f64,         // +DI directional indicator
    pub minus_di: f64,        // -DI directional indicator
    pub cci: f64,             // Commodity Channel Index
    pub williams_r: f64,      // Williams %R (-100 to 0)
    pub vwap: f64,            // Volume Weighted Average Price
    pub vwap_deviation: f64,  // (price - vwap) / vwap
}

// ═══════════════════════════════════════════════════════════════════════════════
// Main DebateLayer
// ═══════════════════════════════════════════════════════════════════════════════

pub struct DebateLayer {
    state: SharedState,
}

impl DebateLayer {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Run the full multi-round adversarial debate.
    ///
    /// This is the central decision-making engine. It:
    /// 1. Assembles intelligence from all sources into a DebateContext
    /// 2. Runs Round 1 (Bull/Bear proposals)
    /// 3. Runs Round 2 (Adversarial challenges)
    /// 4. Runs Round 3 (Synthesis)
    /// 5. Judge adjudicates the final verdict (debate quality ONLY)
    pub async fn run_debate(
        &self,
        symbol: &str,
        current_price: f64,
    ) -> (DebateVerdict, Option<TradeSignal>) {
        println!("\n╔══ DEBATE LAYER ══╗ Symbol: {} @ {:.2}", symbol, current_price);

        // ── Assemble Intelligence ──────────────────────────────────────────
        let ctx = self.assemble_context(symbol, current_price).await;
        println!(
            "[Debate] Context: regime={} conf={:.1}% rsi={:.0} heat={:.1}% skills=+{}/-{}/{}",
            ctx.regime_label,
            ctx.confluence * 100.0,
            ctx.rsi,
            ctx.portfolio_heat * 100.0,
            ctx.skill_votes_bullish,
            ctx.skill_votes_bearish,
            ctx.skill_votes_neutral
        );

        // ── Round 1: Propose ───────────────────────────────────────────────
        println!("[Debate] ── Round 1: Propose ──");
        let bull_proposal = self.round1_bull_propose(&ctx).await;
        let bear_proposal = self.round1_bear_propose(&ctx).await;

        println!(
            "[Debate] Bull: {} (conf {:.2}) | Bear: {} (conf {:.2})",
            bull_proposal.action,
            bull_proposal.confidence,
            bear_proposal.action,
            bear_proposal.confidence
        );

        // ── Round 2: Adversarial ───────────────────────────────────────────
        println!("[Debate] ── Round 2: Adversarial ──");
        let bull_challenge = self.round2_adversarial(&bear_proposal, &ctx, "Bull").await;
        let bear_challenge = self.round2_adversarial(&bull_proposal, &ctx, "Bear").await;

        println!(
            "[Debate] Bull counters: {} | Bear counters: {}",
            bull_challenge.action, bear_challenge.action
        );

        // ── Round 3: Synthesis ─────────────────────────────────────────────
        println!("[Debate] ── Round 3: Synthesis ──");
        let synthesis = self.round3_synthesize(
            &bull_proposal,
            &bear_proposal,
            &bull_challenge,
            &bear_challenge,
            &ctx,
        ).await;

        println!(
            "[Debate] Synthesis: {} (conf {:.2})",
            synthesis.action, synthesis.confidence
        );

        // ── Judge Adjudication ─────────────────────────────────────────────
        println!("[Debate] ── Judge Adjudication ──");
        let verdict = self.judge_adjudicate(&synthesis, &ctx).await;

        println!(
            "[Debate] Judge verdict: {} (conf {:.2}) veto={} rounds={}",
            verdict.action, verdict.confidence, verdict.judge_veto, verdict.rounds_played
        );
        println!("[Debate] Reasoning: {}", &verdict.reasoning[..verdict.reasoning.len().min(200)]);
        println!("╚════════════════════╝");

        // ── Build Signal from Verdict ──────────────────────────────────────
        let signal = if verdict.action == "BUY" || verdict.action == "SELL" {
            Some(self.build_signal_from_verdict(&verdict, &ctx).await)
        } else {
            None
        };

        (verdict, signal)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Context Assembly
    // ═══════════════════════════════════════════════════════════════════════

    async fn assemble_context(&self, symbol: &str, price: f64) -> DebateContext {
        let bars = {
            let hist = self.state.ohlcv_history.read().await;
            hist.get(symbol).cloned().unwrap_or_default()
        };

        let rsi = crate::helpers::compute_rsi(&bars, 14);
        let (_, _, macd_hist) = crate::helpers::compute_macd(&bars);
        let atr_pct = if bars.len() >= 14 {
            let mut tr_sum = 0.0;
            for bar in bars.iter().skip(1) {
                tr_sum += (bar.high - bar.low).abs();
            }
            tr_sum / bars.len() as f64 / price
        } else {
            0.015
        };

        let (portfolio_heat, consecutive_losses) = {
            let p = self.state.portfolio.read().await;
            let heat = if p.total_equity > 0.0 {
                p.open_positions.iter().map(|pos| pos.risk_amount).sum::<f64>() / p.total_equity
            } else {
                0.0
            };
            (heat, p.consecutive_losses)
        };

        let regime = *self.state.market_regime.read().await;
        let regime_label = match &regime {
            Some(MarketRegime::TrendingBull) => "TrendingBull",
            Some(MarketRegime::TrendingBear) => "TrendingBear",
            Some(MarketRegime::Ranging) => "Ranging",
            Some(MarketRegime::Volatile) => "Volatile",
            Some(MarketRegime::LowLiquidity) => "LowLiquidity",
            None => "Unknown",
        }.to_string();

        let confluence = {
            let agg = self.state.last_aggregated_signal.read().await;
            agg.as_ref().map(|a| a.conviction).unwrap_or(0.5)
        };

        let (news_available, patterns_count) = {
            let news = self.state.latest_news.read().await;
            let patterns = self.state.last_patterns.read().await;
            let pc = patterns.get(symbol).map(|p| p.len()).unwrap_or(0);
            (news.contains_key(symbol), pc)
        };

        let (vector_memory_matches, skill_votes_bullish, skill_votes_bearish, skill_votes_neutral) = {
            let vm = self.state.vector_memory.lock().await;
            let vm_count = if vm.is_empty() { 0 } else { 3 }; // approximate
            let votes = self.state.last_skill_votes.read().await;
            let (mut bull, mut bear, mut neut) = (0usize, 0usize, 0usize);
            for v in votes.iter() {
                match v.direction {
                    tredo_core::agent::SkillDirection::Bullish => bull += 1,
                    tredo_core::agent::SkillDirection::Bearish => bear += 1,
                    _ => neut += 1,
                }
            }
            (vm_count, bull, bear, neut)
        };

        let aggregated_signal = {
            let agg = self.state.last_aggregated_signal.read().await;
            agg.clone()
        };

        // Read new indicators from MarketMetricsMeter's latest_metrics snapshot
        let (obv_direction, adx, plus_di, minus_di, cci, williams_r, vwap, vwap_deviation) = {
            let metrics = self.state.latest_metrics.read().await;
            if let Some(snap) = metrics.get(symbol) {
                (snap.obv_direction, snap.adx, snap.plus_di, snap.minus_di,
                 snap.cci, snap.williams_r, snap.vwap, snap.vwap_deviation)
            } else {
                // Fallback: compute directly from bars if metrics not yet available
                let (_obv_raw, obv_dir) = crate::helpers::compute_obv(&bars);
                let (adx_val, pdi, mdi) = crate::helpers::compute_adx(&bars, 14);
                let cci_val = crate::helpers::compute_cci(&bars, 20);
                let wr = crate::helpers::compute_williams_r(&bars, 14);
                let (vwap_val, vwap_dev_val) = crate::helpers::compute_vwap(&bars);
                (obv_dir, adx_val, pdi, mdi, cci_val, wr, vwap_val, vwap_dev_val)
            }
        };

        DebateContext {
            symbol: symbol.to_string(),
            price,
            regime: regime.unwrap_or(MarketRegime::Ranging),
            regime_label,
            confluence,
            rsi,
            macd_hist,
            atr_pct,
            portfolio_heat,
            consecutive_losses,
            news_available,
            patterns_count,
            vector_memory_matches,
            aggregated_signal,
            skill_votes_bullish,
            skill_votes_bearish,
            skill_votes_neutral,
            obv_direction,
            adx,
            plus_di,
            minus_di,
            cci,
            williams_r,
            vwap,
            vwap_deviation,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Round 1: Propose (Bull/Bear teams independently build their case)
    // ═══════════════════════════════════════════════════════════════════════

    async fn round1_bull_propose(&self, ctx: &DebateContext) -> DebateRoundOutput {
        let mut evidence = EvidenceBuilder::new(&ctx.regime_label);

        // 1. Skill consensus (highest weight — aggregates 7 skills)
        if let Some(ref agg) = ctx.aggregated_signal {
            if agg.is_bullish(None) {
                evidence.add("skill_consensus_bullish", 0.6, 0.25);
            } else if agg.is_bearish(None) {
                evidence.add("skill_consensus_bearish", -0.4, 0.25);
            } else {
                evidence.add("skill_consensus_neutral", 0.0, 0.15);
            }
        }

        // 2. Regime alignment
        let regime_score = match ctx.regime {
            MarketRegime::TrendingBull => 0.5,
            MarketRegime::TrendingBear => -0.3,
            MarketRegime::Ranging => 0.1,
            MarketRegime::Volatile => -0.1,
            MarketRegime::LowLiquidity => -0.4,
        };
        evidence.add(&format!("regime={}", ctx.regime_label), regime_score, 0.20);

        // 3. RSI (oversold = bullish opportunity)
        let rsi_score = if ctx.rsi < 30.0 { 0.6 } else if ctx.rsi > 70.0 { -0.5 } else { (50.0 - ctx.rsi) / 100.0 };
        evidence.add(&format!("rsi={:.0}", ctx.rsi), rsi_score, 0.15);

        // 4. News availability (bull team argues: no bad news = good news)
        if ctx.news_available {
            evidence.add("news_available", 0.15, 0.10);
        }

        // 5. Pattern confluence
        if ctx.patterns_count >= 2 {
            evidence.add(&format!("patterns={}", ctx.patterns_count), 0.3, 0.10);
        }

        // 6. MACD momentum
        let macd_score = if ctx.macd_hist > 0.0 { 0.3 } else { -0.2 };
        evidence.add(&format!("macd_hist={:.4}", ctx.macd_hist), macd_score, 0.10);

        // 7. Memory
        if ctx.vector_memory_matches > 0 {
            evidence.add("vector_memory_available", 0.15, 0.10);
        }

        // 8. OBV — volume confirms price trend
        if ctx.obv_direction > 0.0 {
            evidence.add("obv_bullish_volume", 0.35, 0.12);
        } else if ctx.obv_direction < -0.05 {
            evidence.add("obv_bearish_volume", -0.25, 0.12);
        }

        // 9. ADX — trend strength (>25 = strong trend, +DI > -DI = bullish)
        if ctx.adx > 25.0 && ctx.plus_di > ctx.minus_di {
            evidence.add("adx_strong_bull_trend", 0.4, 0.10);
        } else if ctx.adx > 25.0 && ctx.minus_di > ctx.plus_di {
            evidence.add("adx_strong_bear_trend", -0.3, 0.10);
        }

        // 10. CCI — momentum (>100 = strong bullish, <-100 = oversold bounce)
        if ctx.cci > 100.0 {
            evidence.add("cci_strong_bullish", 0.35, 0.08);
        } else if ctx.cci < -100.0 {
            evidence.add("cci_oversold_bounce", 0.25, 0.08);
        }

        // 11. Williams %R — oversold bounce opportunity
        if ctx.williams_r < -80.0 {
            evidence.add("wr_oversold_bullish", 0.3, 0.08);
        } else if ctx.williams_r > -20.0 {
            evidence.add("wr_overbought_caution", -0.2, 0.08);
        }

        // 12. VWAP — price above VWAP = institutional buying
        if ctx.vwap_deviation > 0.002 {
            evidence.add("vwap_above_institutional_buy", 0.3, 0.08);
        }

        let net = evidence.net_score();
        let (bull, bear, _) = evidence.signal_agreement();
        let action = if net > 0.25 && bull > bear { "BUY" } else { "HOLD" };

        DebateRoundOutput {
            participant: "BullTeam".to_string(),
            action: action.to_string(),
            confidence: (0.5 + net * 0.4).min(0.9),
            evidence,
            reasoning: format!("Bull case: {} bullish signals, {} bearish. Net={:.3}. {}", bull, bear, net,
                if action == "BUY" { "Entry justified by regime + skill consensus + momentum." } else { "Insufficient bullish evidence." }),
            key_challenges: vec![
                "Risk of entering at RSI overbought levels".to_string(),
                "Bear regime could invalidate technical signals".to_string(),
            ],
        }
    }

    async fn round1_bear_propose(&self, ctx: &DebateContext) -> DebateRoundOutput {
        let mut evidence = EvidenceBuilder::new(&ctx.regime_label);

        // 1. Skill consensus
        if let Some(ref agg) = ctx.aggregated_signal {
            if agg.is_bullish(None) {
                evidence.add("skill_consensus_bullish", 0.3, 0.20); // Bear team acknowledges but downplays
            } else if agg.is_bearish(None) {
                evidence.add("skill_consensus_bearish", -0.5, 0.25);
            } else {
                evidence.add("skill_consensus_neutral", -0.1, 0.20);
            }
        }

        // 2. Regime risk
        let regime_score = match ctx.regime {
            MarketRegime::TrendingBull => 0.2, // Bear team acknowledges bull trend
            MarketRegime::TrendingBear => -0.6,
            MarketRegime::Ranging => -0.2,
            MarketRegime::Volatile => -0.4,
            MarketRegime::LowLiquidity => -0.5,
        };
        evidence.add(&format!("regime_risk={}", ctx.regime_label), regime_score, 0.25);

        // 3. Portfolio heat (risk management)
        if ctx.portfolio_heat > 0.05 {
            evidence.add(&format!("heat={:.1}%", ctx.portfolio_heat * 100.0), -0.4, 0.20);
        } else if ctx.portfolio_heat > 0.03 {
            evidence.add(&format!("heat={:.1}%", ctx.portfolio_heat * 100.0), -0.15, 0.15);
        }

        // 4. Consecutive losses (fatigue)
        if ctx.consecutive_losses >= 2 {
            evidence.add(&format!("losses={}", ctx.consecutive_losses), -0.3, 0.15);
        }

        // 5. RSI overbought
        if ctx.rsi > 65.0 {
            evidence.add(&format!("rsi_overbought={:.0}", ctx.rsi), -0.25, 0.10);
        }

        // 6. No news = uncertainty
        if !ctx.news_available {
            evidence.add("no_news_uncertainty", -0.15, 0.10);
        }

        // 7. OBV — volume confirms bearish pressure
        if ctx.obv_direction < -0.05 {
            evidence.add("obv_bearish_volume", -0.35, 0.12);
        } else if ctx.obv_direction > 0.0 {
            evidence.add("obv_bullish_volume_conflict", 0.15, 0.12); // Bear acknowledges but downplays
        }

        // 8. ADX — strong downtrend confirms bear case
        if ctx.adx > 25.0 && ctx.minus_di > ctx.plus_di {
            evidence.add("adx_strong_bear_trend", -0.4, 0.10);
        } else if ctx.adx > 25.0 && ctx.plus_di > ctx.minus_di {
            evidence.add("adx_bull_trend_challenge", 0.2, 0.10); // Bear team acknowledges but challenges sustainability
        }

        // 9. CCI — overbought exhaustion or bearish momentum
        if ctx.cci > 100.0 {
            evidence.add("cci_overbought_exhaustion", -0.3, 0.08);
        } else if ctx.cci < -100.0 {
            evidence.add("cci_bearish_momentum", -0.35, 0.08);
        }

        // 10. Williams %R — overbought = distribution signal
        if ctx.williams_r > -20.0 {
            evidence.add("wr_overbought_distribution", -0.3, 0.08);
        } else if ctx.williams_r < -80.0 {
            evidence.add("wr_oversold_challenge", 0.15, 0.08); // Bear team acknowledges oversold
        }

        // 11. VWAP — price below VWAP = institutional selling
        if ctx.vwap_deviation < -0.002 {
            evidence.add("vwap_below_institutional_sell", -0.3, 0.08);
        }

        let net = evidence.net_score();
        let (bull, bear, _) = evidence.signal_agreement();

        // BearTeam proposes SELL when evidence is strongly bearish
        let action = if net < -0.35 && bear > bull + 1 {
            "SELL"
        } else if net < -0.25 || bear > bull {
            "HOLD"
        } else {
            "HOLD"
        };
        let bear_confidence = if action == "SELL" {
            (0.6 + net.abs() * 0.3).min(0.9)
        } else {
            (0.5 + net.abs() * 0.3).min(0.85)
        };

        DebateRoundOutput {
            participant: "BearTeam".to_string(),
            action: action.to_string(),
            confidence: bear_confidence,
            evidence,
            reasoning: format!("Bear case: {} bearish signals, {} bullish. Net={:.3}. {}",
                bear, bull, net,
                if action == "SELL" {
                    "Strong bearish conviction — proposing SHORT."
                } else if net < -0.25 {
                    "Strong case against entry — risk outweighs reward."
                } else {
                    "Weak bear case — no strong reason to block."
                }),
            key_challenges: vec![
                "Bull team ignores portfolio heat building".to_string(),
                "Current regime doesn't support aggressive entry".to_string(),
                "Consecutive losses suggest strategy needs cooling off".to_string(),
            ],
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Round 2: Adversarial (each team challenges the other's proposal)
    // ═══════════════════════════════════════════════════════════════════════

    async fn round2_adversarial(
        &self,
        opposing: &DebateRoundOutput,
        ctx: &DebateContext,
        team: &str,
    ) -> DebateRoundOutput {
        let mut evidence = EvidenceBuilder::new(&ctx.regime_label);
        let mut challenges = Vec::new();

        // Challenge the opposing team's key evidence
        for ev in &opposing.evidence.evidences {
            if ev.score.abs() > 0.2 {
                if ev.score > 0.0 && team == "Bear" {
                    evidence.add(&format!("challenge_{}", ev.factor), -ev.score * 0.5, 0.3);
                    challenges.push(format!("Challenging {} (score {:.2}): context suggests this may be overconfident", ev.factor, ev.score));
                } else if ev.score < 0.0 && team == "Bull" {
                    evidence.add(&format!("challenge_{}", ev.factor), -ev.score * 0.5, 0.3);
                    challenges.push(format!("Challenging {} (score {:.2}): risk is priced in by current levels", ev.factor, ev.score));
                }
            }
        }

        // Add counter-evidence specific to the challenge
        if team == "Bear" {
            if ctx.consecutive_losses >= 2 {
                evidence.add("unaddressed_losses", -0.25, 0.20);
                challenges.push(format!("Bull team ignores {} consecutive losses", ctx.consecutive_losses));
            }
            if ctx.portfolio_heat > 0.06 {
                evidence.add("unaddressed_heat", -0.2, 0.15);
                challenges.push(format!("Bull team ignores portfolio heat at {:.1}%", ctx.portfolio_heat * 100.0));
            }
            // Bear team challenges bullish volume — OBV diverges from price
            if ctx.obv_direction > 0.0 && ctx.macd_hist < 0.0 {
                evidence.add("obv_price_divergence_bearish", -0.2, 0.15);
                challenges.push(format!("OBV shows bullish volume but MACD is negative — volume/price divergence"));
            }
            // Bear team: ADX confirms strong downtrend
            if ctx.adx > 25.0 && ctx.minus_di > ctx.plus_di {
                evidence.add("adx_bear_trend_unaddressed", -0.2, 0.15);
                challenges.push(format!("ADX {:.0} confirms downtrend (+DI {:.0} < -DI {:.0})", ctx.adx, ctx.plus_di, ctx.minus_di));
            }
            // Bear team: VWAP below shows institutional selling
            if ctx.vwap_deviation < -0.002 {
                evidence.add("vwap_institutional_selling_unaddressed", -0.2, 0.15);
                challenges.push(format!("Price {:.1}% below VWAP — institutional selling pressure", ctx.vwap_deviation * 100.0));
            }
        } else {
            if let Some(ref agg) = ctx.aggregated_signal {
                if agg.is_bullish(None) && agg.conviction > 0.6 {
                    evidence.add("strong_skill_consensus_unaddressed", 0.3, 0.20);
                    challenges.push("Bear team ignores strong bullish skill consensus with high conviction".to_string());
                }
            }
            if ctx.rsi < 35.0 {
                evidence.add("oversold_unaddressed", 0.25, 0.15);
                challenges.push(format!("Bear team ignores RSI at {:.0} (oversold)", ctx.rsi));
            }
            // Bull team: OBV confirms bullish volume flow
            if ctx.obv_direction > 0.0 {
                evidence.add("obv_bullish_volume_unaddressed", 0.2, 0.15);
                challenges.push(format!("Bear team ignores bullish OBV trend (dir={:.3})", ctx.obv_direction));
            }
            // Bull team: ADX shows strong uptrend
            if ctx.adx > 25.0 && ctx.plus_di > ctx.minus_di {
                evidence.add("adx_bull_trend_unaddressed", 0.25, 0.15);
                challenges.push(format!("ADX {:.0} confirms uptrend (+DI {:.0} > -DI {:.0})", ctx.adx, ctx.plus_di, ctx.minus_di));
            }
            // Bull team: CCI oversold = bounce opportunity
            if ctx.cci < -100.0 {
                evidence.add("cci_oversold_unaddressed", 0.2, 0.15);
                challenges.push(format!("CCI at {:.0} — deeply oversold, bounce likely", ctx.cci));
            }
            // Bull team: Williams %R oversold = bounce signal
            if ctx.williams_r < -80.0 {
                evidence.add("wr_oversold_unaddressed", 0.2, 0.15);
                challenges.push(format!("Williams %R at {:.0} — oversold, bounce signal ignored", ctx.williams_r));
            }
        }

        let net = evidence.net_score();
        let action = if team == "Bull" && net > 0.15 { "COUNTER" } else if team == "Bear" && net < -0.15 { "COUNTER" } else { "WEAKENED" };

        DebateRoundOutput {
            participant: format!("{}Counter", team),
            action: action.to_string(),
            confidence: (0.4 + net.abs() * 0.3).min(0.8),
            evidence,
            reasoning: format!("{} counters: {} challenges. Net={:.3}", team, challenges.len(), net),
            key_challenges: challenges,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Round 3: Synthesis (merge both cases into balanced verdict)
    // ═══════════════════════════════════════════════════════════════════════

    async fn round3_synthesize(
        &self,
        bull_proposal: &DebateRoundOutput,
        bear_proposal: &DebateRoundOutput,
        bull_challenge: &DebateRoundOutput,
        bear_challenge: &DebateRoundOutput,
        ctx: &DebateContext,
    ) -> DebateRoundOutput {
        let mut evidence = EvidenceBuilder::new(&ctx.regime_label);

        let bull_net = bull_proposal.evidence.net_score();
        let bear_net = bear_proposal.evidence.net_score();
        let bull_counter_net = bull_challenge.evidence.net_score();
        let bear_counter_net = bear_challenge.evidence.net_score();

        // Original proposal evidence (weight 0.30 each)
        evidence.add("bull_proposal", bull_net, 0.30);
        evidence.add("bear_proposal", bear_net, 0.30);

        // Challenge evidence — successful challenges reduce confidence
        evidence.add("bull_counter", bull_counter_net, 0.20);
        evidence.add("bear_counter", bear_counter_net, 0.20);

        // Adversarial impact
        let bull_wounded = bull_challenge.action == "WEAKENED";
        let bear_wounded = bear_challenge.action == "WEAKENED";
        if bull_wounded && bull_counter_net < -0.1 {
            evidence.add("bull_weakened_by_adversarial", -0.15, 0.15);
        }
        if bear_wounded && bear_counter_net > 0.1 {
            evidence.add("bear_weakened_by_adversarial", 0.15, 0.15);
        }

        // Regime tiebreaker
        let regime_tiebreaker = match ctx.regime {
            MarketRegime::TrendingBull => 0.1,
            MarketRegime::TrendingBear => -0.1,
            MarketRegime::Ranging => 0.0,
            MarketRegime::Volatile => -0.05,
            MarketRegime::LowLiquidity => -0.1,
        };
        evidence.add("regime_tiebreaker", regime_tiebreaker, 0.10);

        let net = evidence.net_score();
        let (bull_evidence, bear_evidence, _) = evidence.signal_agreement();

        // Synthesis verdict
        let action = if net > 0.20 && bull_evidence > bear_evidence {
            "BUY"
        } else if net < -0.20 && bear_evidence > bull_evidence {
            "HOLD"
        } else {
            "HOLD" // Mixed signals = HOLD (conservative)
        };

        let confidence = if action == "BUY" {
            (0.5 + net * 0.3).min(0.9)
        } else {
            (0.5 + net.abs() * 0.2).min(0.8)
        };

        let challenges_summary = [
            bull_proposal.key_challenges.clone(),
            bear_proposal.key_challenges.clone(),
        ].concat();

        DebateRoundOutput {
            participant: "Synthesizer".to_string(),
            action: action.to_string(),
            confidence,
            evidence,
            reasoning: format!(
                "Synthesis: Bull proposal net={:.3}, Bear proposal net={:.3}. Counters: bull={:.3}, bear={:.3}. Net verdict={:.3}. {}",
                bull_net, bear_net, bull_counter_net, bear_counter_net, net,
                if action == "BUY" {
                    "Bull case outweighs bear concerns after adversarial rounds."
                } else {
                    "Bear concerns outweigh bull case, or signals too mixed for confident entry."
                }
            ),
            key_challenges: challenges_summary,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Judge: Adjudication (debate quality ONLY — no risk/regime re-checks)
    // ═══════════════════════════════════════════════════════════════════════
    //
    // The Judge's ONLY job is to evaluate whether the debate evidence
    // supports a confident action. It does NOT re-run risk checks,
    // regime checks, confluence checks, or R:R calculations.
    // Those are all enforced by the HardRulesGate (Layer 1).
    //
    // The Judge can veto ONLY based on:
    // - Debate quality: low confidence from the synthesis
    // - Evidence contradiction: bull and bear evidence are equally strong
    // - Insufficient evidence: not enough signals to form a conviction
    //
    // This separation of concerns prevents the Judge from duplicating
    // the gate's work and ensures each layer has a single responsibility.

    async fn judge_adjudicate(
        &self,
        synthesis: &DebateRoundOutput,
        ctx: &DebateContext,
    ) -> DebateVerdict {
        let mut veto = false;
        let mut veto_reason = String::new();

        // ── Judge Veto Conditions (debate quality ONLY) ────────────────────
        // The Judge evaluates whether the debate produced a high-quality signal.
        // Risk/regime/confluence are NOT checked here — the gate handled those.

        // 1. Minimum confidence threshold (regime-adaptive)
        //    If synthesis confidence is too low, the debate didn't produce enough conviction.
        let min_confidence = match ctx.regime {
            MarketRegime::TrendingBull => 0.40,
            MarketRegime::TrendingBear => 0.60,
            MarketRegime::Ranging => 0.50,
            MarketRegime::Volatile => 0.65,
            MarketRegime::LowLiquidity => 0.75,
        };
        if synthesis.action == "BUY" && synthesis.confidence < min_confidence {
            veto = true;
            veto_reason = format!(
                "VETO: Confidence {:.0}% below regime minimum {:.0}% — insufficient debate conviction",
                synthesis.confidence * 100.0,
                min_confidence * 100.0
            );
        }

        // 2. Evidence contradiction: bull and bear evidence are too close
        //    If the teams are evenly matched, the Judge should not force a decision.
        let bull_net = synthesis.evidence.evidences.iter()
            .filter(|e| e.factor.starts_with("bull"))
            .map(|e| e.score)
            .sum::<f64>();
        let bear_net = synthesis.evidence.evidences.iter()
            .filter(|e| e.factor.starts_with("bear"))
            .map(|e| e.score)
            .sum::<f64>();
        let evidence_gap = (bull_net - bear_net).abs();
        if synthesis.action == "BUY" && evidence_gap < 0.10 {
            veto = true;
            veto_reason = format!(
                "VETO: Bull/Bear evidence gap only {:.3} — teams too evenly matched for confident entry",
                evidence_gap
            );
        }

        // 3. Insufficient signal count: fewer than 3 evidence factors
        //    A thin debate (few signals) produces unreliable verdicts.
        let total_factors = synthesis.evidence.evidences.len();
        if synthesis.action == "BUY" && total_factors < 3 {
            veto = true;
            veto_reason = format!(
                "VETO: Only {} evidence factors — insufficient data for confident decision",
                total_factors
            );
        }

        let final_action = if veto { "HOLD" } else { &synthesis.action };
        let final_confidence = if veto { 0.9 } else { synthesis.confidence };

        let reasoning = if veto {
            format!(
                "JUDGE VETO: {}. Override to HOLD. Synthesis was {} (conf {:.0}%).",
                veto_reason, synthesis.action, synthesis.confidence * 100.0
            )
        } else {
            format!(
                "JUDGE APPROVE: {} (conf {:.0}%). Regime={}, heat={:.1}%, confluence={:.1}%. Debate evidence validated.",
                synthesis.action, synthesis.confidence * 100.0, ctx.regime_label,
                ctx.portfolio_heat * 100.0, ctx.confluence * 100.0
            )
        };

        if veto {
            println!("[Judge] ⛔ {}", veto_reason);
        } else {
            println!("[Judge] ✅ Approved: {} (conf {:.0}%)", synthesis.action, synthesis.confidence * 100.0);
        }

        DebateVerdict {
            action: final_action.to_string(),
            confidence: final_confidence,
            reasoning,
            evidence_summary: synthesis.evidence.summary(),
            rounds_played: 3,
            judge_veto: veto,
            appeal_used: false,
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Build TradeSignal from verdict
    // ═══════════════════════════════════════════════════════════════════════

    async fn build_signal_from_verdict(
        &self,
        verdict: &DebateVerdict,
        ctx: &DebateContext,
    ) -> TradeSignal {
        let direction = if verdict.action == "BUY" {
            tredo_core::TradeDirection::Long
        } else {
            tredo_core::TradeDirection::Short
        };

        // Use autonomous level computation (agent decides its own levels)
        let rules = self.state.rules.read().await;
        let patterns_for_levels = {
            let pats = self.state.last_patterns.read().await;
            pats.get(&ctx.symbol).cloned().unwrap_or_default()
        };

        let pivots = tredo_core::calculate_pivot_points(
            ctx.price * 1.01, ctx.price * 0.99, ctx.price * 0.998, rules.pivot_method,
        );

        let (entry, stop_loss, take_profit, _rule_rr) =
            crate::helpers::compute_autonomous_levels(
                &ctx.symbol, ctx.price, &pivots, &patterns_for_levels,
                ctx.regime, ctx.rsi, ctx.macd_hist, ctx.atr_pct,
                &rules, ctx.aggregated_signal.as_ref(),
            );

        // Adaptive position sizing
        let (equity, effective_risk) = {
            let portfolio = self.state.portfolio.read().await;
            let eq = portfolio.cash_balance
                + portfolio.open_positions.iter().map(|p| p.current_price * p.quantity).sum::<f64>();
            let conf_mult = (verdict.confidence / 0.7).min(1.2).max(0.5);
            let loss_mult = if ctx.consecutive_losses >= 3 { 0.5 } else if ctx.consecutive_losses >= 2 { 0.7 } else { 1.0 };
            let heat_mult = if ctx.portfolio_heat > 0.08 { 0.5 } else if ctx.portfolio_heat > 0.05 { 0.7 } else { 1.0 };
            let regime_mult = match ctx.regime {
                MarketRegime::TrendingBull => 1.0,
                MarketRegime::TrendingBear => 0.7,
                MarketRegime::Ranging => 0.8,
                _ => 0.6,
            };
            let mult = (conf_mult * loss_mult * heat_mult * regime_mult).clamp(0.3, 1.2);
            (eq, (rules.max_risk_per_trade * mult).max(0.003))
        };

        let position_size = crate::helpers::calculate_position_size(equity, effective_risk, entry, stop_loss);

        let final_rr = {
            let risk = (entry - stop_loss).abs();
            let reward = (take_profit - entry).abs();
            if risk > 0.0 { reward / risk } else { 2.0 }
        };

        // Store reasoning
        {
            let mut last_reason = self.state.last_llm_reason.write().await;
            *last_reason = format!("DebateLayer: {}", verdict.reasoning);
        }

        println!(
            "[DebateLayer] Signal: {} {} @ entry={:.2} SL={:.2} TP={:.2} (RR {:.1}:1, size {:.4})",
            verdict.action, ctx.symbol, entry, stop_loss, take_profit, final_rr, position_size
        );

        TradeSignal {
            symbol: ctx.symbol.clone(),
            direction,
            entry_price: entry,
            stop_loss,
            take_profit,
            position_size,
            confidence_score: verdict.confidence.min(0.95),
            confluence_score: ctx.confluence,
            risk_reward_ratio: final_rr,
            reasoning: verdict.reasoning.clone(),
            timestamp: chrono::Utc::now(),
            session_valid: true,
            risk_check_passed: !verdict.judge_veto,
        }
    }
}
