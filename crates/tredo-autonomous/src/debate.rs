// Smart Debate System — Evidence-based multi-agent debate with regime-adaptive thresholds.
// Each agent now accumulates ranked evidence factors instead of single threshold checks.
// No LLM dependency — all intelligence is rule-based but adaptive.

use crate::state::SharedState;
use crate::{
    correlation_checker::CorrelationChecker, on_chain_data::OnChainData,
    regime_classifier::RegimeClassifier, sentiment_analyzer::SentimentAnalyzer,
    volatility_calculator::VolatilityCalculator,
};
use tredo_core::{AgentInput, MarketContext};

// ═══════════════════════════════════════════════════════════════════════════════
// EvidenceBuilder — structured evidence accumulation for all agents
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct Evidence {
    pub factor: String,
    pub score: f64, // positive = bullish, negative = bearish
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct EvidenceBuilder {
    pub evidences: Vec<Evidence>,
    pub regime: String,
}

impl EvidenceBuilder {
    pub fn new(regime: &str) -> Self {
        Self {
            evidences: Vec::new(),
            regime: regime.to_string(),
        }
    }

    pub fn add(&mut self, factor: &str, score: f64, weight: f64) {
        self.evidences.push(Evidence {
            factor: factor.to_string(),
            score,
            weight,
        });
    }

    /// Weighted sum of all evidence scores
    pub fn net_score(&self) -> f64 {
        let total_weight: f64 = self.evidences.iter().map(|e| e.weight).sum();
        if total_weight <= 0.0 {
            return 0.0;
        }
        self.evidences
            .iter()
            .map(|e| e.score * e.weight)
            .sum::<f64>()
            / total_weight
    }

    /// Number of positive vs negative evidence factors
    pub fn signal_agreement(&self) -> (usize, usize, usize) {
        let bullish = self.evidences.iter().filter(|e| e.score > 0.1).count();
        let bearish = self.evidences.iter().filter(|e| e.score < -0.1).count();
        let neutral = self.evidences.len() - bullish - bearish;
        (bullish, bearish, neutral)
    }

    /// Confidence: more agreement = higher confidence
    pub fn agreement_confidence(&self) -> f64 {
        let (bull, bear, _neu) = self.signal_agreement();
        let total = (bull + bear + _neu) as f64;
        if total == 0.0 {
            return 0.0;
        }
        let agreement = (bull.max(bear) as f64) / total;
        // Scale: 50% agreement = 0.5 confidence, 100% = 0.95
        0.5 + (agreement * 0.45)
    }

    /// Format evidence summary for COT storage
    pub fn summary(&self) -> String {
        let (bull, bear, neu) = self.signal_agreement();
        format!(
            "net={:.3} bull={} bear={} neutral={}",
            self.net_score(),
            bull,
            bear,
            neu
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Regime-adaptive thresholds
// ═══════════════════════════════════════════════════════════════════════════════

/// Returns the regime-specific net_score threshold needed to propose BUY.
/// Trending markets are easier to enter (lower bar), volatile markets are harder.
pub fn regime_buy_threshold(regime: &str) -> f64 {
    match regime {
        "TrendingBull" => 0.25,
        "TrendingBear" => 0.55,
        "Ranging" => 0.40,
        "Volatile" => 0.50,
        _ => 0.40, // LowLiquidity, Neutral
    }
}

/// Returns the regime-specific vol threshold for blocking.
/// Volatile markets tolerate higher vol, trending markets block earlier.
pub fn regime_vol_block_threshold(regime: &str) -> f64 {
    match regime {
        "TrendingBull" => 0.04,
        "TrendingBear" => 0.03,
        "Ranging" => 0.025,
        "Volatile" => 0.05, // Volatile regime tolerates its own vol
        _ => 0.03,
    }
}

/// Returns the final buy_score threshold for the debate to trigger a BUY.
pub fn regime_debate_buy_threshold(regime: &str) -> f64 {
    match regime {
        "TrendingBull" => 0.45,
        "TrendingBear" => 0.65,
        "Ranging" => 0.55,
        "Volatile" => 0.70,
        _ => 0.55,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// DebateTurn — lightweight turn for debate participants
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct DebateTurn {
    pub action: String,
    pub confidence: f64,
    pub reasoning: String,
    pub evidence: EvidenceBuilder,
}

fn extract_context(input: &AgentInput) -> Option<&MarketContext> {
    match input {
        AgentInput::ConfluenceRequest { context } => Some(context),
        AgentInput::RiskRequest { context } => Some(context),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ProposerAgent — multi-signal evidence-based proposal
// ═══════════════════════════════════════════════════════════════════════════════

pub struct ProposerAgent {
    state: SharedState,
}
pub struct CriticAgent {
    state: SharedState,
}
pub struct RiskAgent {
    state: SharedState,
}
pub struct HistorianAgent {
    state: SharedState,
}

impl ProposerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn propose(&self, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "HOLD".into(),
                    confidence: 0.0,
                    reasoning: "no context".into(),
                    evidence: EvidenceBuilder::new("unknown"),
                }
            }
        };

        // Gather all signals first
        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!("proposer action on {} regime conf confluence", ctx.symbol),
                3,
            )
            .await;

        let sentiment = SentimentAnalyzer::new(self.state.clone())
            .analyze_sentiment(&ctx.symbol)
            .await;
        let (_vol, vol_expansion) = VolatilityCalculator::new(self.state.clone())
            .compute_volatility(&ctx.symbol, ctx.current_price)
            .await;
        let regime = RegimeClassifier::new(self.state.clone())
            .detect_regime(&ctx.symbol, ctx.current_price)
            .await;
        let onchain = OnChainData::new(self.state.clone())
            .fetch_onchain(&ctx.symbol)
            .await;

        // Pull additional context from state for richer evidence
        let regime_str = format!("{:?}", regime);
        let (confluence, rsi, patterns_count) = {
            let agg = self.state.last_aggregated_signal.read().await;
            let metrics = self.state.latest_metrics.read().await;
            let patterns = self.state.last_patterns.read().await;
            let m = metrics.get(&ctx.symbol);
            let pats = patterns.get(&ctx.symbol).map(|p| p.len()).unwrap_or(0);
            let conf = agg.as_ref().map(|a| a.conviction).unwrap_or(0.5);
            let rsi = m.map(|m| m.rsi_14).unwrap_or(50.0);
            (conf, rsi, pats)
        };

        // === EVIDENCE-BASED DECISION ===
        let mut evidence = EvidenceBuilder::new(&regime_str);

        // 1. Sentiment signal (0-1 scale, 0.5 = neutral)
        let sent_score = (sentiment - 0.5) * 2.0; // normalize to -1..+1
        evidence.add(&format!("sentiment={:.2}", sentiment), sent_score, 0.20);

        // 2. On-chain accumulation/distribution
        let onchain_score = (onchain - 0.5) * 2.0;
        evidence.add(&format!("onchain={:.2}", onchain), onchain_score, 0.15);

        // 3. Regime alignment
        let regime_score = if regime_str.contains("Bull") {
            0.6
        } else if regime_str.contains("Bear") {
            -0.5
        } else if regime_str.contains("Volatile") {
            -0.2 // Volatile = uncertainty
        } else {
            0.0 // Ranging/Neutral
        };
        evidence.add(&format!("regime={}", regime_str), regime_score, 0.20);

        // 4. RSI extremes (mean reversion signals)
        let rsi_score = if rsi < 30.0 {
            0.5 // Oversold = bullish
        } else if rsi > 70.0 {
            -0.5 // Overbought = bearish
        } else {
            (50.0 - rsi) / 100.0 // Mild bias
        };
        evidence.add(&format!("rsi={:.0}", rsi), rsi_score, 0.15);

        // 5. Pattern confluence (more patterns = stronger signal)
        let pattern_score = if patterns_count >= 3 {
            0.3
        } else if patterns_count >= 1 {
            0.1
        } else {
            -0.1
        };
        evidence.add(&format!("patterns={}", patterns_count), pattern_score, 0.10);

        // 6. Skill consensus (from MarketIntelligence aggregation)
        let agg_score = (confluence - 0.5) * 2.0;
        evidence.add(
            &format!("skill_consensus={:.2}", confluence),
            agg_score,
            0.20,
        );

        // 7. Memory bonus/penalty (trained recall interpretation)
        let memory_score =
            if trained_recall.contains("high regret") || trained_recall.contains("regret") {
                -0.3 // Past similar setups had bad outcomes
            } else if trained_recall.contains("profit") || trained_recall.contains("good") {
                0.2 // Past similar setups were profitable
            } else {
                0.0 // No strong memory signal
            };
        if memory_score != 0.0 {
            evidence.add("memory_pattern", memory_score, 0.15);
        }

        // 8. Volatility expansion warning
        if vol_expansion {
            evidence.add("vol_expansion", -0.2, 0.10);
        }

        // === DECIDE ===
        let net = evidence.net_score();
        let threshold = regime_buy_threshold(&regime_str);
        let (bull, bear, _neu) = evidence.signal_agreement();
        let agreement = evidence.agreement_confidence();

        let action = if net > threshold && bull > bear {
            "BUY"
        } else {
            "HOLD"
        };

        let confidence = if action == "BUY" {
            (0.5 + net * 0.3 + agreement * 0.2).min(0.95)
        } else {
            (0.3 + agreement * 0.2).min(0.7)
        };

        println!(
            "[Proposer] {} for {} | net={:.3} threshold={:.2} bull={} bear={} regime={}",
            action, ctx.symbol, net, threshold, bull, bear, regime_str
        );

        DebateTurn {
            action: action.to_string(),
            confidence,
            reasoning: format!(
                "Proposer {}: net={:.3} (threshold={:.2}). {} bullish {} bearish signals. Regime={}. {}",
                action, net, threshold, bull, bear, regime_str,
                if !trained_recall.contains("No strong") {
                    format!("Memory: {}", &trained_recall[..trained_recall.len().min(120)])
                } else {
                    "No strong memory pattern".to_string()
                }
            ),
            evidence,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CriticAgent — multi-factor critique
// ═══════════════════════════════════════════════════════════════════════════════

impl CriticAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn critique(&self, proposal: &str, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "CRITIQUE".into(),
                    confidence: 0.5,
                    reasoning: "no context".into(),
                    evidence: EvidenceBuilder::new("unknown"),
                }
            }
        };

        let trained_recall = self
            .state
            .recall_trained_memory(
                &format!("critic on proposal {} for {}", proposal, ctx.symbol),
                2,
            )
            .await;

        // === MULTI-FACTOR CRITIQUE ===
        let regime_str = format!("{:?}", {
            let r = self.state.market_regime.read().await;
            *r.as_ref().unwrap_or(&crate::types::MarketRegime::Ranging)
        });

        let corr = CorrelationChecker::new(self.state.clone())
            .check_correlation(&ctx.symbol)
            .await;

        let mut evidence = EvidenceBuilder::new(&regime_str);
        let mut concerns = Vec::new();
        let mut validations = Vec::new();

        // 1. Correlation check
        if corr < 0.4 {
            evidence.add(&format!("low_corr={:.2}", corr), -0.4, 0.25);
            concerns.push(format!("low correlation ({:.2}) — possible fakeout", corr));
        } else if corr > 0.7 {
            evidence.add(&format!("strong_corr={:.2}", corr), 0.3, 0.25);
            validations.push(format!("strong correlation ({:.2})", corr));
        } else {
            evidence.add(&format!("moderate_corr={:.2}", corr), 0.0, 0.15);
        }

        // 2. Volume-price divergence check
        let bars = {
            let hist = self.state.ohlcv_history.read().await;
            hist.get(&ctx.symbol).cloned().unwrap_or_default()
        };
        if bars.len() >= 10 {
            let recent_vol: f64 = bars.iter().rev().take(5).map(|b| b.volume).sum::<f64>() / 5.0;
            let older_vol: f64 = bars
                .iter()
                .rev()
                .skip(5)
                .take(5)
                .map(|b| b.volume)
                .sum::<f64>()
                / 5.0;
            let price_up = bars.last().unwrap().close > bars[bars.len() - 5].close;

            if older_vol > 0.0 {
                let vol_ratio = recent_vol / older_vol;
                // Bearish divergence: price rising but volume declining
                if price_up && vol_ratio < 0.7 {
                    evidence.add("bearish_vol_divergence", -0.3, 0.20);
                    concerns.push(format!(
                        "price rising on declining volume (vol ratio {:.2})",
                        vol_ratio
                    ));
                }
                // Bullish: price rising with volume confirmation
                else if price_up && vol_ratio > 1.3 {
                    evidence.add("bullish_vol_confirmation", 0.3, 0.20);
                    validations.push(format!(
                        "volume confirms price move (ratio {:.2})",
                        vol_ratio
                    ));
                }
                // Bearish: price falling on high volume
                else if !price_up && vol_ratio > 1.3 {
                    evidence.add("bearish_volume_selling", -0.3, 0.20);
                    concerns.push(format!("selling pressure (volume ratio {:.2})", vol_ratio));
                }
            }
        }

        // 3. Regime proposal mismatch
        if proposal == "BUY" && regime_str.contains("Bear") {
            evidence.add("proposal_regime_mismatch", -0.4, 0.20);
            concerns.push("BUY proposal contradicts bearish regime".to_string());
        } else if proposal == "BUY" && regime_str.contains("Bull") {
            evidence.add("proposal_regime_aligned", 0.2, 0.15);
            validations.push("BUY aligned with bullish regime".to_string());
        }

        // 4. RSI exhaustion check
        let rsi = {
            let m = self.state.latest_metrics.read().await;
            m.get(&ctx.symbol).map(|m| m.rsi_14).unwrap_or(50.0)
        };
        if proposal == "BUY" && rsi > 75.0 {
            evidence.add(&format!("rsi_overbought={:.0}", rsi), -0.3, 0.15);
            concerns.push(format!("RSI at {:.0} — overbought risk", rsi));
        }

        // 5. Memory-based critique
        if trained_recall.contains("fakeout") || trained_recall.contains("whipsaw") {
            evidence.add("memory_fakeout_pattern", -0.3, 0.15);
            concerns.push("memory shows similar setups led to fakeouts".to_string());
        }

        // === DECIDE ===
        let net = evidence.net_score();
        let concern_count = concerns.len();
        let validation_count = validations.len();

        let action = if concern_count > validation_count && concern_count >= 2 {
            "BLOCK"
        } else if concern_count > 0 {
            "CAUTION"
        } else {
            "OK"
        };

        let confidence = if action == "BLOCK" {
            0.85
        } else if action == "CAUTION" {
            0.65
        } else {
            0.55
        };

        println!(
            "[Critic] {} proposal={} | concerns={} validations={} net={:.3}",
            action, proposal, concern_count, validation_count, net
        );

        let reasoning = format!(
            "Critic on {}: {} | {} concerns, {} validations. Net={:.3}. Concerns: [{}]. Validations: [{}]. {}",
            proposal,
            action,
            concern_count,
            validation_count,
            net,
            concerns.join("; "),
            validations.join("; "),
            if !trained_recall.contains("No strong") {
                format!("Memory: {}", &trained_recall[..trained_recall.len().min(80)])
            } else {
                String::new()
            }
        );

        DebateTurn {
            action: action.to_string(),
            confidence,
            reasoning,
            evidence,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// RiskAgent — regime-adaptive risk assessment
// ═══════════════════════════════════════════════════════════════════════════════

impl RiskAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn assess(&self, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "PASS".into(),
                    confidence: 0.5,
                    reasoning: "no context".into(),
                    evidence: EvidenceBuilder::new("unknown"),
                }
            }
        };

        let trained_recall = self
            .state
            .recall_trained_memory(&format!("risk assessment for {} vol", ctx.symbol), 2)
            .await;

        let regime_str = format!("{:?}", {
            let r = self.state.market_regime.read().await;
            *r.as_ref().unwrap_or(&crate::types::MarketRegime::Ranging)
        });

        let (vol, vol_expansion) = VolatilityCalculator::new(self.state.clone())
            .compute_volatility(&ctx.symbol, ctx.current_price)
            .await;

        // === MULTI-FACTOR RISK ASSESSMENT ===
        let mut evidence = EvidenceBuilder::new(&regime_str);
        let mut risk_factors = Vec::new();

        // 1. Volatility vs regime-adaptive threshold
        let vol_threshold = regime_vol_block_threshold(&regime_str);
        if vol > vol_threshold {
            evidence.add(
                &format!("vol={:.3}>threshold={:.3}", vol, vol_threshold),
                -0.5,
                0.30,
            );
            risk_factors.push(format!(
                "volatility {:.3} exceeds regime threshold {:.3}",
                vol, vol_threshold
            ));
        } else {
            evidence.add(
                &format!("vol={:.3}<threshold={:.3}", vol, vol_threshold),
                0.2,
                0.25,
            );
        }

        // 2. Volatility expansion
        if vol_expansion {
            evidence.add("vol_expansion_detected", -0.3, 0.15);
            risk_factors.push("volatility expansion detected — regime transition risk".to_string());
        }

        // 3. Portfolio heat (how much risk is already deployed)
        let portfolio = self.state.portfolio.read().await;
        let portfolio_heat = {
            let total_risk: f64 = portfolio.open_positions.iter().map(|p| p.risk_amount).sum();
            if portfolio.total_equity > 0.0 {
                total_risk / portfolio.total_equity
            } else {
                0.0
            }
        };
        if portfolio_heat > 0.08 {
            evidence.add(
                &format!("high_heat={:.1}%", portfolio_heat * 100.0),
                -0.4,
                0.20,
            );
            risk_factors.push(format!(
                "portfolio heat at {:.1}% — near safety limit",
                portfolio_heat * 100.0
            ));
        } else if portfolio_heat > 0.05 {
            evidence.add(
                &format!("moderate_heat={:.1}%", portfolio_heat * 100.0),
                -0.15,
                0.15,
            );
        } else {
            evidence.add("low_heat", 0.15, 0.10);
        }

        // 4. Consecutive losses (fatigue detection)
        if portfolio.consecutive_losses >= 3 {
            evidence.add(
                &format!("consecutive_losses={}", portfolio.consecutive_losses),
                -0.4,
                0.20,
            );
            risk_factors.push(format!(
                "{} consecutive losses — high fatigue risk",
                portfolio.consecutive_losses
            ));
        } else if portfolio.consecutive_losses >= 2 {
            evidence.add(
                &format!("consecutive_losses={}", portfolio.consecutive_losses),
                -0.15,
                0.10,
            );
        }

        // 5. Regime-specific risk: volatile regimes are inherently riskier
        if regime_str.contains("Volatile") {
            evidence.add("volatile_regime", -0.2, 0.10);
            risk_factors.push("volatile regime — elevated baseline risk".to_string());
        } else if regime_str.contains("TrendingBull") {
            evidence.add("bull_trend", 0.15, 0.10);
        }

        // 6. Memory-based risk
        if trained_recall.contains("regret") || trained_recall.contains("loss") {
            evidence.add("memory_risk_signal", -0.2, 0.10);
            risk_factors.push("memory shows similar setups had high regret".to_string());
        }

        // === DECIDE ===
        let net = evidence.net_score();
        let (risk_count, _safe_count, _neu) = evidence.signal_agreement();

        // BLOCK if: multiple risk factors AND net risk is negative
        let action = if risk_count >= 3 && net < -0.2 {
            "BLOCK"
        } else if risk_count >= 2 || net < -0.3 {
            "CAUTION"
        } else {
            "PASS"
        };

        let confidence = if action == "BLOCK" {
            0.90
        } else if action == "CAUTION" {
            0.70
        } else {
            0.60
        };

        println!(
            "[Risk] {} | vol={:.3} heat={:.1}% losses={} regime={} net={:.3}",
            action,
            vol,
            portfolio_heat * 100.0,
            portfolio.consecutive_losses,
            regime_str,
            net
        );

        DebateTurn {
            action: action.to_string(),
            confidence,
            reasoning: format!(
                "Risk {}: vol={:.3} (threshold={:.3}) heat={:.1}% losses={} regime={}. Factors: [{}]",
                action,
                vol,
                vol_threshold,
                portfolio_heat * 100.0,
                portfolio.consecutive_losses,
                regime_str,
                risk_factors.join("; ")
            ),
            evidence,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// HistorianAgent — interpretive memory analysis
// ═══════════════════════════════════════════════════════════════════════════════

impl HistorianAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn recall(&self, input: &AgentInput) -> DebateTurn {
        let ctx = match extract_context(input) {
            Some(c) => c,
            None => {
                return DebateTurn {
                    action: "RECALL".into(),
                    confidence: 0.5,
                    reasoning: "no context".into(),
                    evidence: EvidenceBuilder::new("unknown"),
                }
            }
        };

        let regime_str = format!("{:?}", {
            let r = self.state.market_regime.read().await;
            *r.as_ref().unwrap_or(&crate::types::MarketRegime::Ranging)
        });

        // === VECTOR MEMORY SEARCH ===
        let mut similar_episodes = Vec::new();
        {
            let vm = self.state.vector_memory.read().await;
            if !vm.is_empty() {
                let query = format!(
                    "{} {} price={:.2}",
                    ctx.symbol, "historical outcome", ctx.current_price
                );
                let llm_for_search = (*self.state.llm).clone();
                if let Ok(results) = vm.search(&query, 3, &llm_for_search).await {
                    similar_episodes = results;
                }
            }
        }

        // === AGENTMEMORY SEARCH ===
        let mem = tredo_core::AgentMemoryClient::new();
        let past = mem
            .recall(&format!("trained lessons OR past decisions {}", ctx.symbol))
            .await
            .unwrap_or_default();

        // === INTERPRETIVE ANALYSIS (not just raw text) ===
        let mut evidence = EvidenceBuilder::new(&regime_str);

        if !similar_episodes.is_empty() {
            // Count profitable vs losing episodes
            let profitable = similar_episodes
                .iter()
                .filter(|r| r.regret_score.map(|s| s < 0.1).unwrap_or(true))
                .count();
            let losing = similar_episodes
                .iter()
                .filter(|r| r.regret_score.map(|s| s >= 0.3).unwrap_or(false))
                .count();
            let total = similar_episodes.len();

            let win_rate = if total > 0 {
                profitable as f64 / total as f64
            } else {
                0.5
            };

            if win_rate > 0.6 {
                evidence.add(
                    &format!("similar_win_rate={:.0}%", win_rate * 100.0),
                    0.3,
                    0.30,
                );
            } else if win_rate < 0.4 {
                evidence.add(
                    &format!("similar_win_rate={:.0}%", win_rate * 100.0),
                    -0.3,
                    0.30,
                );
            } else {
                evidence.add("similar_win_rate_mixed", 0.0, 0.10);
            }

            // Average regret of similar episodes
            let avg_regret: f64 = similar_episodes
                .iter()
                .filter_map(|r| r.regret_score)
                .sum::<f64>()
                / total.max(1) as f64;
            if avg_regret > 0.4 {
                evidence.add(&format!("avg_regret={:.2}", avg_regret), -0.25, 0.20);
            } else if avg_regret < 0.15 {
                evidence.add(&format!("avg_regret={:.2}", avg_regret), 0.2, 0.20);
            }

            println!(
                "[Historian] {} similar episodes: {} profitable, {} losing, avg_regret={:.2}",
                total, profitable, losing, avg_regret
            );
        }

        // Memory-based lessons
        if past.len() > 3 {
            evidence.add("strong_memory_baseline", 0.15, 0.15);
        }

        // === BUILD INTERPRETIVE REASONING ===
        let (bull, bear, _neu) = evidence.signal_agreement();
        let net = evidence.net_score();

        let reasoning = if !similar_episodes.is_empty() {
            let summaries: Vec<String> = similar_episodes
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let regret = r
                        .regret_score
                        .map(|s| format!("regret={:.2}", s))
                        .unwrap_or("unknown".into());
                    format!(
                        "  {}. {} (sim={:.0}%, {})",
                        i + 1,
                        &r.summary_text[..r.summary_text.len().min(80)],
                        r.similarity * 100.0,
                        regret
                    )
                })
                .collect();

            format!(
                "Historian: {} similar episodes found. {} profitable, {} losing. Net={:.3}. Pattern: {}. Episodes:\n{}",
                similar_episodes.len(),
                bull,
                bear,
                net,
                if net > 0.1 {
                    "past similar setups were profitable — supports entry"
                } else if net < -0.1 {
                    "past similar setups had losses — warns against entry"
                } else {
                    "mixed historical results — neutral"
                },
                summaries.join("\n")
            )
        } else {
            format!(
                "Historian: no similar episodes found for {}. Default caution on limited data.",
                ctx.symbol
            )
        };

        let confidence = if similar_episodes.len() >= 3 {
            (0.6 + (net.abs() * 0.2)).min(0.9)
        } else {
            0.5
        };

        DebateTurn {
            action: "RECALL".to_string(),
            confidence,
            reasoning,
            evidence,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// run_debate — regime-adaptive aggregation
// ═══════════════════════════════════════════════════════════════════════════════

pub async fn run_debate(
    state: SharedState,
    input: &AgentInput,
    aggregated_signal: Option<&tredo_core::AggregatedSignal>,
) -> (String, f64, String, Vec<DebateTurn>) {
    let proposer = ProposerAgent::new(state.clone());
    let critic = CriticAgent::new(state.clone());
    let risk = RiskAgent::new(state.clone());
    let historian = HistorianAgent::new(state.clone());

    let prop = proposer.propose(input).await;
    let crit = critic.critique(&prop.action, input).await;
    let rsk = risk.assess(input).await;
    let hist = historian.recall(input).await;

    let turns = vec![prop.clone(), crit.clone(), rsk.clone(), hist.clone()];

    // === REGIME-ADAPTIVE SCORING ===
    let regime_str = prop.evidence.regime.clone();
    let buy_threshold = regime_debate_buy_threshold(&regime_str);

    let mut buy_score = 0.0;

    // Proposer contribution (weighted by its evidence agreement)
    if prop.action == "BUY" {
        buy_score += prop.confidence * 0.25;
    }

    // Critic contribution (structured: OK/CAUTION/BLOCK)
    match crit.action.as_str() {
        "OK" => buy_score += 0.15,
        "CAUTION" => buy_score -= 0.10, // Caution reduces score instead of ignoring
        "BLOCK" => buy_score -= 0.30,
        _ => {}
    }

    // Risk contribution
    match rsk.action.as_str() {
        "PASS" => buy_score += rsk.confidence * 0.25,
        "CAUTION" => buy_score -= 0.05,
        "BLOCK" => buy_score -= 0.40,
        _ => {}
    }

    // Historian contribution (interpretive: profitable vs losing pattern)
    buy_score += hist.evidence.net_score() * 0.15;

    // Cross-skill aggregated consensus from MarketIntelligence
    if let Some(agg) = aggregated_signal {
        let before = buy_score;
        if agg.is_bullish(None) {
            buy_score += (agg.net_signal.abs() * 0.30).min(0.30);
        } else if agg.is_bearish(None) {
            buy_score -= (agg.net_signal.abs() * 0.30).min(0.30);
        }
        buy_score += (agg.conviction - 0.3).max(0.0) * 0.15;

        let delta = buy_score - before;
        if delta.abs() > 0.005 {
            println!(
                "[Debate] AggregatedSignal influence: {:+.3} (net={:+.2}, conv={:.0}%)",
                delta,
                agg.net_signal,
                agg.conviction * 100.0
            );
        }
    }

    // === REGIME-ADAPTIVE FINAL DECISION ===
    let risk_blocks = rsk.action == "BLOCK" || crit.action == "BLOCK";

    let (final_action, conf, reason) = if buy_score > buy_threshold && !risk_blocks {
        (
            "BUY".to_string(),
            buy_score.min(0.95),
            format!(
                "Debate BUY: score={:.3} > threshold={:.2} | Prop={} Crit={} Risk={} Hist={} | Regime={}",
                buy_score, buy_threshold, prop.action, crit.action, rsk.action,
                hist.action, regime_str
            ),
        )
    } else if buy_score < 0.2 || risk_blocks {
        (
            "HOLD".to_string(),
            0.85,
            format!(
                "Debate HOLD: score={:.3} blocks=[risk={},crit={}] regime={}",
                buy_score, rsk.action, crit.action, regime_str
            ),
        )
    } else {
        (
            "HOLD".to_string(),
            0.65,
            format!(
                "Debate HOLD: score={:.3} (mixed) threshold={:.2} regime={}",
                buy_score, buy_threshold, regime_str
            ),
        )
    };

    println!(
        "[Debate] {} conf={:.3} | score={:.3} threshold={:.2} regime={}",
        final_action, conf, buy_score, buy_threshold, regime_str
    );

    (final_action, conf, reason, turns)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Degradation policy + types
// ═══════════════════════════════════════════════════════════════════════════════

pub fn execute_conservative_degradation_policy(missing_skills_count: usize) -> DebateOutcome {
    if missing_skills_count > 2 {
        return DebateOutcome {
            signal_approved: false,
            final_direction: "HOLD".to_string(),
            entry_level: 0.0,
            adjusted_stop_loss: 0.0,
            adjusted_take_profit: 0.0,
            calculated_confidence: 0.0,
            consensus_justification:
                "CRITICAL DATA DEGRADATION: More than 2 skills failed. Trading paused.".to_string(),
        };
    }

    DebateOutcome {
        signal_approved: true,
        final_direction: "HOLD".to_string(),
        entry_level: 0.0,
        adjusted_stop_loss: 0.0,
        adjusted_take_profit: 0.0,
        calculated_confidence: 0.1,
        consensus_justification: format!(
            "PARTIAL DEGRADATION ({} skills missing). Defaulting to HOLD.",
            missing_skills_count
        ),
    }
}

#[derive(Clone, Debug)]
pub struct DebateOutcome {
    pub signal_approved: bool,
    pub final_direction: String,
    pub entry_level: f64,
    pub adjusted_stop_loss: f64,
    pub adjusted_take_profit: f64,
    pub calculated_confidence: f64,
    pub consensus_justification: String,
}
