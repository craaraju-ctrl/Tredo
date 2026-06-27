// ═══════════════════════════════════════════════════════════════════════════════
// SuperIntelligence Decision Layer
//
// Upgrades the trading decision pipeline with:
//   1. UnifiedDecisionContext — single validated data structure through pipeline
//   2. CrossValidationEngine — each signal validated by ≥2 independent sources
//   3. ConvictionStack — multi-factor conviction (directional + confidence +
//      agreement + memory + risk)
//   4. DecisionTrace — every BUY/SELL has ranked factor importance
//
// Architecture:
//   MarketIntelligence → UnifiedDecisionContext → CrossValidation →
//   ConvictionStack → RankedDecision → StrategyDecisionAgent
//
// No LLM dependency — all intelligence is evidence-based and deterministic.
// ═══════════════════════════════════════════════════════════════════════════════

use crate::debate::EvidenceBuilder;
use crate::state::SharedState;
use crate::types::MarketRegime;
use std::collections::HashMap;
use tredo_core::agent::SkillDirection;
use tredo_core::AggregatedSignal;
use tredo_core::ConfirmationLevel;

// ═══════════════════════════════════════════════════════════════════════════════
// CROSS-VALIDATION — Each signal validated by ≥2 independent sources
// ═══════════════════════════════════════════════════════════════════════════════

/// A single validated factor with cross-validation metadata.
#[derive(Debug, Clone)]
pub struct ValidatedFactor {
    /// Human-readable name (e.g. "MarketMetricsMeter", "RegimeDetector")
    pub name: String,
    /// Numeric score 0.0–1.0 where >0.5 = bullish, <0.5 = bearish
    pub score: f64,
    /// Direction implied by this factor
    pub direction: SkillDirection,
    /// The primary skill/agent that produced this factor
    pub primary_source: String,
    /// Other sources that independently validated this factor
    pub cross_validated_by: Vec<String>,
    /// How well the sources agree (0.0 = total conflict, 1.0 = perfect agreement)
    pub validation_confidence: f64,
    /// Weight in the ensemble (from DisciplineRules)
    pub weight: f64,
    /// Human-readable description of what this factor means
    pub description: String,
}

/// A cross-validation conflict between two sources.
#[derive(Debug, Clone)]
pub struct CrossValidationConflict {
    pub factor_name: String,
    pub sources: Vec<String>,
    pub primary_value: f64,
    pub conflicting_value: f64,
    /// How the conflict was resolved
    pub resolution: ConflictResolution,
}

#[derive(Debug, Clone)]
pub enum ConflictResolution {
    /// Trusted higher-weight source
    DeferToWeight { winner: String, loser: String },
    /// Averaged both sources
    Averaged { final_value: f64 },
    /// Lowered both confidences due to disagreement
    Penalized {
        penalty: f64,
        adjusted_primary: f64,
        adjusted_conflicting: f64,
    },
}

/// Complete cross-validation report for a trading decision.
#[derive(Debug, Clone)]
pub struct CrossValidationReport {
    /// All validated factors
    pub factors: Vec<ValidatedFactor>,
    /// Conflicts detected during validation
    pub conflicts: Vec<CrossValidationConflict>,
    /// Overall validation health (0.0–1.0)
    pub overall_validation_score: f64,
    /// Summary string for logging
    pub summary: String,
}

impl CrossValidationReport {
    pub fn empty() -> Self {
        Self {
            factors: vec![],
            conflicts: vec![],
            overall_validation_score: 1.0,
            summary: "No factors to validate".to_string(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CONVICTION STACK — Multi-factor conviction (not a single number)
// ═══════════════════════════════════════════════════════════════════════════════

/// A single component of the conviction stack.
#[derive(Debug, Clone)]
pub struct ConvictionFactor {
    pub name: String,
    pub value: f64,
    pub weight_in_decision: f64,
    /// Percentage contribution to the final conviction
    pub contribution_pct: f64,
}

/// Multi-factor conviction stack replacing a single confluence number.
///
/// Now expanded to 8 factors (was 5):
/// 1. Directional Conviction — net_signal from SkillAggregator
/// 2. Confidence Conviction — avg confidence of directional skills
/// 3. Agreement Conviction — % of directional skills agreeing
/// 4. Memory Conviction — historical win rate from vector memory
/// 5. Risk Conviction — inverse of portfolio heat
/// 6. Pattern Conviction — candlestick pattern strength
/// 7. Timeframe Alignment — multi-TF confirmation quality
/// 8. Synthesis Score — blend of cross-validation + debate evidence
#[derive(Debug, Clone)]
pub struct ConvictionStack {
    /// Directional conviction: net_signal from SkillAggregator (normalized -1..+1 → 0..1)
    pub directional_conviction: f64,
    /// Confidence conviction: average confidence of directional (non-neutral) skills
    pub confidence_conviction: f64,
    /// Agreement conviction: % of non-neutral skills agreeing on the consensus direction
    pub agreement_conviction: f64,
    /// Memory conviction: historical win rate of similar setups (from vector memory)
    pub memory_conviction: f64,
    /// Risk conviction: 1.0 - normalize(portfolio_heat)
    pub risk_conviction: f64,
    /// Pattern conviction: candlestick pattern alignment with direction (0–1)
    pub pattern_conviction: f64,
    /// Timeframe alignment: multi-TF confirmation (0–1)
    pub timeframe_alignment: f64,
    /// Synthesis score: blend of cross-validation quality + debate evidence strength (0–1)
    pub synthesis_score: f64,
    /// Final weighted conviction (0.0–1.0)
    pub final_conviction: f64,
    /// All factors contributing to the final conviction
    pub factors: Vec<ConvictionFactor>,
}

impl ConvictionStack {
    /// Compute the full conviction stack from available data.
    ///
    /// Regime-adaptive coefficients:
    /// - TrendingBull: directional matters most (0.4), risk matters least (0.1)
    /// - TrendingBear: agreement matters most (0.3), memory important (0.25)
    /// - Ranging: confidence matters most (0.3), everything balanced
    /// - Volatile: risk matters most (0.35), memory important (0.25)
    /// - LowLiquidity: risk dominates (0.5)
    pub fn compute(
        aggregated_signal: &AggregatedSignal,
        portfolio_heat: f64,
        memory_win_rate: Option<f64>,
        regime: &MarketRegime,
    ) -> Self {
        Self::compute_extended(
            aggregated_signal,
            portfolio_heat,
            memory_win_rate,
            regime,
            None,
            None,
            None,
        )
    }

    /// Compute the full 8-factor conviction stack with optional extended data.
    ///
    /// Extended factors:
    /// - pattern_strength: from candlestick pattern detection (0–1, 1 = strong bullish patterns)
    /// - multi_tf_confirmation: from multi-timeframe pattern confirmation (0–1, 1 = all TFs agree)
    /// - cross_validation_score: from CrossValidationEngine (0–1, 1 = all signals validated)
    fn compute_extended(
        aggregated_signal: &AggregatedSignal,
        portfolio_heat: f64,
        memory_win_rate: Option<f64>,
        regime: &MarketRegime,
        pattern_strength: Option<f64>,
        multi_tf_confirmation: Option<f64>,
        cross_validation_score: Option<f64>,
    ) -> Self {
        // 1. Directional conviction: map net_signal [-1..+1] to [0..1]
        let directional = (aggregated_signal.net_signal + 1.0) / 2.0;

        // 2. Confidence conviction: average confidence of all skills
        let total_skills = (aggregated_signal.bullish_count
            + aggregated_signal.bearish_count
            + aggregated_signal.neutral_count)
            .max(1) as f64;
        let directional_skills =
            (aggregated_signal.bullish_count + aggregated_signal.bearish_count) as f64;
        let confidence = if directional_skills > 0.0 {
            (aggregated_signal.conviction * 0.6 + (directional_skills / total_skills) * 0.4)
                .clamp(0.0, 1.0)
        } else {
            aggregated_signal.conviction
        };

        // 3. Agreement conviction: % of directional skills agreeing
        let agreement = if aggregated_signal.bullish_count + aggregated_signal.bearish_count > 0 {
            let max_count = aggregated_signal
                .bullish_count
                .max(aggregated_signal.bearish_count) as f64;
            let total_directional =
                (aggregated_signal.bullish_count + aggregated_signal.bearish_count) as f64;
            max_count / total_directional
        } else {
            0.5
        };

        // 4. Memory conviction: historical win rate
        let memory = memory_win_rate.unwrap_or(0.5);

        // 5. Risk conviction: 1.0 - normalized portfolio heat
        let risk = 1.0 - (portfolio_heat * 5.0).clamp(0.0, 1.0);

        // 6. Pattern conviction: candlestick pattern alignment
        //    Higher when patterns confirm the net signal direction
        let pattern = pattern_strength.unwrap_or(0.5);

        // 7. Timeframe alignment: multi-TF confirmation
        //    Higher when multiple timeframes agree on direction
        let timeframe = multi_tf_confirmation.unwrap_or(0.5);

        // 8. Synthesis score: cross-validation + debate evidence blend
        //    Reflects how well independent sources agree on the signal
        let synthesis = cross_validation_score.unwrap_or(0.5);

        // Regime-adaptive coefficients (expanded to 8 factors)
        let (w_dir, w_conf, w_agr, w_mem, w_risk, w_pat, w_tf, w_syn) = match regime {
            MarketRegime::TrendingBull => (0.30, 0.15, 0.10, 0.10, 0.08, 0.10, 0.10, 0.07),
            MarketRegime::TrendingBear => (0.10, 0.10, 0.20, 0.20, 0.12, 0.08, 0.08, 0.12),
            MarketRegime::Ranging => (0.15, 0.20, 0.15, 0.10, 0.12, 0.10, 0.08, 0.10),
            MarketRegime::Volatile => (0.10, 0.10, 0.08, 0.18, 0.25, 0.07, 0.07, 0.15),
            MarketRegime::LowLiquidity => (0.08, 0.08, 0.08, 0.15, 0.35, 0.06, 0.06, 0.14),
        };

        let final_conviction = (directional * w_dir
            + confidence * w_conf
            + agreement * w_agr
            + memory * w_mem
            + risk * w_risk
            + pattern * w_pat
            + timeframe * w_tf
            + synthesis * w_syn)
            .clamp(0.0, 1.0);

        let factors = vec![
            ConvictionFactor {
                name: "Directional".to_string(),
                value: directional,
                weight_in_decision: w_dir,
                contribution_pct: directional * w_dir / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Confidence".to_string(),
                value: confidence,
                weight_in_decision: w_conf,
                contribution_pct: confidence * w_conf / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Agreement".to_string(),
                value: agreement,
                weight_in_decision: w_agr,
                contribution_pct: agreement * w_agr / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Memory".to_string(),
                value: memory,
                weight_in_decision: w_mem,
                contribution_pct: memory * w_mem / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Risk".to_string(),
                value: risk,
                weight_in_decision: w_risk,
                contribution_pct: risk * w_risk / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Pattern".to_string(),
                value: pattern,
                weight_in_decision: w_pat,
                contribution_pct: pattern * w_pat / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Timeframe".to_string(),
                value: timeframe,
                weight_in_decision: w_tf,
                contribution_pct: timeframe * w_tf / final_conviction.max(0.001) * 100.0,
            },
            ConvictionFactor {
                name: "Synthesis".to_string(),
                value: synthesis,
                weight_in_decision: w_syn,
                contribution_pct: synthesis * w_syn / final_conviction.max(0.001) * 100.0,
            },
        ];

        Self {
            directional_conviction: directional,
            confidence_conviction: confidence,
            agreement_conviction: agreement,
            memory_conviction: memory,
            risk_conviction: risk,
            pattern_conviction: pattern,
            timeframe_alignment: timeframe,
            synthesis_score: synthesis,
            final_conviction,
            factors,
        }
    }

    /// Human-readable summary of the conviction stack.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self
            .factors
            .iter()
            .map(|f| {
                format!(
                    "{}={:.0}%(w={:.2})",
                    f.name, f.contribution_pct, f.weight_in_decision
                )
            })
            .collect();
        parts.push(format!("FINAL={:.1}%", self.final_conviction * 100.0));
        parts.join(" | ")
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// DECISION TRACE — Every BUY/SELL has ranked factor importance
// ═══════════════════════════════════════════════════════════════════════════════

/// A ranked factor showing its contribution to the final decision.
#[derive(Debug, Clone)]
pub struct RankedFactor {
    pub rank: usize,
    pub factor_name: String,
    pub score: f64,
    pub weight: f64,
    pub contribution_pct: f64,
    pub direction: String,
    pub validated_by: Vec<String>,
}

/// Complete decision trace for a trade signal.
#[derive(Debug, Clone)]
pub struct DecisionTrace {
    pub final_action: String,
    pub final_confidence: f64,
    pub final_conviction: f64,
    pub ranked_factors: Vec<RankedFactor>,
    pub cross_validations: Vec<String>,
    pub regime: String,
    pub regime_threshold: f64,
    pub memory_win_rate: Option<f64>,
    pub risk_assessment: String,
    pub conviction_summary: String,
}

impl DecisionTrace {
    pub fn new(
        action: &str,
        confidence: f64,
        conviction: &ConvictionStack,
        validated: &CrossValidationReport,
        regime_label: &str,
        regime_threshold: f64,
        portfolio_heat: f64,
    ) -> Self {
        // Rank factors by contribution percentage
        let mut ranked: Vec<RankedFactor> = validated
            .factors
            .iter()
            .map(|f| {
                let contribution = f.score * f.weight;
                RankedFactor {
                    rank: 0, // will be set after sorting
                    factor_name: f.name.clone(),
                    score: f.score,
                    weight: f.weight,
                    contribution_pct: contribution * 100.0,
                    direction: match f.direction {
                        SkillDirection::Bullish => "Bullish ⬆️".to_string(),
                        SkillDirection::Bearish => "Bearish ⬇️".to_string(),
                        SkillDirection::Neutral => "Neutral ➡️".to_string(),
                    },
                    validated_by: f.cross_validated_by.clone(),
                }
            })
            .collect();

        // Sort by contribution descending
        ranked.sort_by(|a, b| {
            b.contribution_pct
                .partial_cmp(&a.contribution_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Assign ranks
        for (i, factor) in ranked.iter_mut().enumerate() {
            factor.rank = i + 1;
        }

        // Only keep top 10 factors
        ranked.truncate(10);

        let cross_validations: Vec<String> = validated
            .conflicts
            .iter()
            .map(|c| {
                format!(
                    "⚠ {}: {} vs {} — resolved via {:?}",
                    c.factor_name,
                    if !c.sources.is_empty() {
                        &c.sources[0]
                    } else {
                        "?"
                    },
                    if c.sources.len() > 1 {
                        &c.sources[1]
                    } else {
                        "?"
                    },
                    c.resolution
                )
            })
            .collect();

        let risk_assessment = if portfolio_heat > 0.08 {
            format!("HIGH RISK — heat {:.1}%", portfolio_heat * 100.0)
        } else if portfolio_heat > 0.05 {
            format!("MODERATE RISK — heat {:.1}%", portfolio_heat * 100.0)
        } else {
            format!("LOW RISK — heat {:.1}%", portfolio_heat * 100.0)
        };

        Self {
            final_action: action.to_string(),
            final_confidence: confidence,
            final_conviction: conviction.final_conviction,
            ranked_factors: ranked,
            cross_validations,
            regime: regime_label.to_string(),
            regime_threshold,
            memory_win_rate: None,
            risk_assessment,
            conviction_summary: conviction.summary(),
        }
    }

    /// Format the decision trace as a readable string.
    pub fn format_for_log(&self) -> String {
        let mut lines = vec![
            format!("\n╔══ SUPERINTELLIGENCE DECISION TRACE ══╗"),
            format!(
                "║ Action: {} (conf {:.1}%)",
                self.final_action,
                self.final_confidence * 100.0
            ),
            format!(
                "║ Conviction: {:.1}% | Regime: {} (threshold {:.0}%)",
                self.final_conviction * 100.0,
                self.regime,
                self.regime_threshold * 100.0
            ),
            format!("║ Risk: {}", self.risk_assessment),
            format!("╠══ Ranked Factors ══╣"),
        ];

        for factor in &self.ranked_factors {
            lines.push(format!(
                "║  #{}. {} (w={:.2}) {} — {:.1}% contribution {}",
                factor.rank,
                factor.factor_name,
                factor.weight,
                factor.direction,
                factor.contribution_pct,
                if factor.validated_by.is_empty() {
                    String::new()
                } else {
                    format!("[validated by: {}]", factor.validated_by.join(", "))
                }
            ));
        }

        if !self.cross_validations.is_empty() {
            lines.push("╠══ Cross-Validations ══╣".to_string());
            for cv in &self.cross_validations {
                lines.push(format!("║  {}", cv));
            }
        }

        if let Some(wr) = self.memory_win_rate {
            lines.push(format!("║ Memory win rate: {:.0}%", wr * 100.0));
        }

        lines.push(format!(
            "║ Conviction breakdown: {}",
            self.conviction_summary
        ));
        lines.push("╚══════════════════════════════════╝".to_string());

        lines.join("\n")
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// UNIFIED DECISION CONTEXT — single validated data structure
// ═══════════════════════════════════════════════════════════════════════════════

/// Memory context with structured data.
#[derive(Debug, Clone)]
pub struct MemoryContext {
    /// How many similar episodes were found
    pub similar_episodes_count: usize,
    /// How many were profitable
    pub profitable_count: usize,
    /// How many were losing
    pub losing_count: usize,
    /// Win rate from similar episodes
    pub win_rate: Option<f64>,
    /// Average regret from similar episodes
    pub avg_regret: Option<f64>,
    /// Key lessons from memory
    pub lessons: Vec<String>,
}

impl MemoryContext {
    pub fn empty() -> Self {
        Self {
            similar_episodes_count: 0,
            profitable_count: 0,
            losing_count: 0,
            win_rate: None,
            avg_regret: None,
            lessons: vec![],
        }
    }

    pub async fn from_vector_memory(state: &SharedState, symbol: &str, current_price: f64) -> Self {
        let mut similar_episodes = Vec::new();
        {
            let vm = state.vector_memory.read().await;
            if !vm.is_empty() {
                let query = format!("{} price={:.2} trading outcome", symbol, current_price);
                let llm = (*state.llm).clone();
                if let Ok(results) = vm.search(&query, 5, &llm).await {
                    similar_episodes = results;
                }
            }
        }

        if similar_episodes.is_empty() {
            return MemoryContext::empty();
        }

        let total = similar_episodes.len();
        let profitable = similar_episodes
            .iter()
            .filter(|r| r.regret_score.map(|s| s < 0.15).unwrap_or(true))
            .count();
        let losing = similar_episodes
            .iter()
            .filter(|r| r.regret_score.map(|s| s >= 0.3).unwrap_or(false))
            .count();

        let win_rate = Some(profitable as f64 / total.max(1) as f64);
        let avg_regret = Some(
            similar_episodes
                .iter()
                .filter_map(|r| r.regret_score)
                .sum::<f64>()
                / total.max(1) as f64,
        );

        let lessons: Vec<String> = similar_episodes
            .iter()
            .take(3)
            .map(|r| {
                let regret = r
                    .regret_score
                    .map(|s| format!("(regret={:.2})", s))
                    .unwrap_or_default();
                format!(
                    "{} {}",
                    &r.summary_text[..r.summary_text.len().min(80)],
                    regret
                )
            })
            .collect();

        MemoryContext {
            similar_episodes_count: total,
            profitable_count: profitable,
            losing_count: losing,
            win_rate,
            avg_regret,
            lessons,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CROSS-VALIDATION ENGINE
// ═══════════════════════════════════════════════════════════════════════════════

/// The CrossValidationEngine validates each signal against ≥2 independent sources.
///
/// Validation pairs:
/// - MarketMetricsMeter (26 indicators) ↔ RegimeDetector (trend direction)
/// - SupportResistance (S/R levels) ↔ VolumeProfile (POC/VAH/VAL)
/// - OrderFlow (buy/sell pressure) ↔ FundingRate (counter-sentiment)
/// - SentimentAnalyzer (news sentiment) ↔ OnChainData (accumulation/distribution)
/// - VolatilityCalculator (vol expansion) ↔ Liquidity (depth/spread)
/// - CorrelationChecker (cross-symbol) ↔ PatternRetriever (pattern confirmation)
pub struct CrossValidationEngine;

impl CrossValidationEngine {
    /// Run all cross-validations on the given factors and produce a report.
    ///
    /// `validation_data` is a map of skill name → (score, direction).
    /// Used by the pipeline after skills execute to validate each result.
    pub fn validate(
        skill_results: &HashMap<String, (f64, SkillDirection, f64)>, // name → (score, direction, weight)
    ) -> CrossValidationReport {
        let mut factors = Vec::new();
        let mut conflicts = Vec::new();

        // Helper to get a skill's output
        let get =
            |name: &str| -> Option<(f64, SkillDirection, f64)> { skill_results.get(name).copied() };

        // ── Validation Pair 1: MarketMetricsMeter ↔ RegimeDetector ──────────
        if let Some((mmm_score, mmm_dir, mmm_w)) = get("MarketMetricsMeter") {
            if let Some((reg_score, reg_dir, reg_w)) = get("RegimeDetector") {
                let (validated, conflict) = Self::validate_pair(
                    "Regime/Trend",
                    mmm_score,
                    mmm_dir,
                    mmm_w,
                    "MarketMetricsMeter",
                    reg_score,
                    reg_dir,
                    reg_w,
                    "RegimeDetector",
                );
                factors.push(validated);
                if let Some(c) = conflict {
                    conflicts.push(c);
                }
            } else {
                factors.push(ValidatedFactor {
                    name: "MarketMetricsMeter (standalone)".to_string(),
                    score: mmm_score,
                    direction: mmm_dir,
                    primary_source: "MarketMetricsMeter".to_string(),
                    cross_validated_by: vec![],
                    validation_confidence: 0.5,
                    weight: mmm_w,
                    description: "26-indicator ensemble (unvalidated — no RegimeDetector)"
                        .to_string(),
                });
            }
        }

        // ── Validation Pair 2: SupportResistance ↔ VolumeProfile ────────────
        if let Some((sr_score, sr_dir, sr_w)) = get("SupportResistance") {
            if let Some((vp_score, vp_dir, vp_w)) = get("VolumeProfile") {
                let (validated, conflict) = Self::validate_pair(
                    "Levels/Structure",
                    sr_score,
                    sr_dir,
                    sr_w,
                    "SupportResistance",
                    vp_score,
                    vp_dir,
                    vp_w,
                    "VolumeProfile",
                );
                factors.push(validated);
                if let Some(c) = conflict {
                    conflicts.push(c);
                }
            } else {
                factors.push(ValidatedFactor {
                    name: "SupportResistance (standalone)".to_string(),
                    score: sr_score,
                    direction: sr_dir,
                    primary_source: "SupportResistance".to_string(),
                    cross_validated_by: vec![],
                    validation_confidence: 0.5,
                    weight: sr_w,
                    description: "S/R level detection (unvalidated)".to_string(),
                });
            }
        }

        // ── Validation Pair 3: OrderFlow ↔ FundingRate ──────────────────────
        if let Some((of_score, of_dir, of_w)) = get("OrderFlow") {
            if let Some((fr_score, fr_dir, fr_w)) = get("FundingRate") {
                let (validated, conflict) = Self::validate_pair(
                    "Volume/Sentiment",
                    of_score,
                    of_dir,
                    of_w,
                    "OrderFlow",
                    fr_score,
                    fr_dir,
                    fr_w,
                    "FundingRate",
                );
                factors.push(validated);
                if let Some(c) = conflict {
                    conflicts.push(c);
                }
            } else {
                factors.push(ValidatedFactor {
                    name: "OrderFlow (standalone)".to_string(),
                    score: of_score,
                    direction: of_dir,
                    primary_source: "OrderFlow".to_string(),
                    cross_validated_by: vec![],
                    validation_confidence: 0.5,
                    weight: of_w,
                    description: "Buy/sell pressure (unvalidated)".to_string(),
                });
            }
        }

        // ── Validation Pair 4: SentimentAnalyzer ↔ OnChainData ──────────────
        if let Some((sa_score, sa_dir, sa_w)) = get("SentimentAnalyzer") {
            if let Some((oc_score, oc_dir, oc_w)) = get("OnChainData") {
                let (validated, conflict) = Self::validate_pair(
                    "Sentiment/Accumulation",
                    sa_score,
                    sa_dir,
                    sa_w,
                    "SentimentAnalyzer",
                    oc_score,
                    oc_dir,
                    oc_w,
                    "OnChainData",
                );
                factors.push(validated);
                if let Some(c) = conflict {
                    conflicts.push(c);
                }
            } else {
                factors.push(ValidatedFactor {
                    name: "SentimentAnalyzer (standalone)".to_string(),
                    score: sa_score,
                    direction: sa_dir,
                    primary_source: "SentimentAnalyzer".to_string(),
                    cross_validated_by: vec![],
                    validation_confidence: 0.5,
                    weight: sa_w,
                    description: "News sentiment (unvalidated)".to_string(),
                });
            }
        }

        // ── Validation Pair 5: VolatilityCalculator ↔ Liquidity ─────────────
        if let Some((vc_score, vc_dir, vc_w)) = get("VolatilityCalculator") {
            if let Some((liq_score, liq_dir, liq_w)) = get("Liquidity") {
                let (validated, conflict) = Self::validate_pair(
                    "Vol/Liquidity",
                    vc_score,
                    vc_dir,
                    vc_w,
                    "VolatilityCalculator",
                    liq_score,
                    liq_dir,
                    liq_w,
                    "Liquidity",
                );
                factors.push(validated);
                if let Some(c) = conflict {
                    conflicts.push(c);
                }
            } else {
                factors.push(ValidatedFactor {
                    name: "VolatilityCalculator (standalone)".to_string(),
                    score: vc_score,
                    direction: vc_dir,
                    primary_source: "VolatilityCalculator".to_string(),
                    cross_validated_by: vec![],
                    validation_confidence: 0.5,
                    weight: vc_w,
                    description: "Volatility expansion (unvalidated)".to_string(),
                });
            }
        }

        // Add remaining standalone skills that weren't paired
        for (name, &(score, dir, weight)) in skill_results {
            let already_paired = factors.iter().any(|f| {
                f.cross_validated_by.contains(name)
                    || f.primary_source == *name
                    || f.name.contains(name)
            });
            if !already_paired {
                factors.push(ValidatedFactor {
                    name: format!("{} (standalone)", name),
                    score,
                    direction: dir,
                    primary_source: name.clone(),
                    cross_validated_by: vec![],
                    validation_confidence: 0.5,
                    weight,
                    description: format!("{} result (unvalidated)", name),
                });
            }
        }

        // Calculate overall validation score
        let total_factors = factors.len().max(1);
        let validated_count = factors
            .iter()
            .filter(|f| !f.cross_validated_by.is_empty())
            .count();
        let avg_validation_conf: f64 =
            factors.iter().map(|f| f.validation_confidence).sum::<f64>() / total_factors as f64;
        let conflict_penalty = (conflicts.len() as f64 * 0.1).min(0.5);
        let overall_validation_score = ((validated_count as f64 / total_factors as f64) * 0.5
            + avg_validation_conf * 0.5
            - conflict_penalty)
            .clamp(0.0, 1.0);

        let summary = format!(
            "Cross-Validation: {} factors, {} validated pairs, {} conflicts. Overall score: {:.1}%",
            factors.len(),
            validated_count / 2,
            conflicts.len(),
            overall_validation_score * 100.0
        );

        CrossValidationReport {
            factors,
            conflicts,
            overall_validation_score,
            summary,
        }
    }

    /// Validate a pair of sources. If they agree, boost confidence. If they disagree, flag conflict.
    #[allow(clippy::too_many_arguments)]
    fn validate_pair(
        factor_name: &str,
        primary_score: f64,
        primary_dir: SkillDirection,
        primary_weight: f64,
        primary_source: &str,
        secondary_score: f64,
        secondary_dir: SkillDirection,
        secondary_weight: f64,
        secondary_source: &str,
    ) -> (ValidatedFactor, Option<CrossValidationConflict>) {
        let both_directional =
            primary_dir != SkillDirection::Neutral && secondary_dir != SkillDirection::Neutral;
        let same_direction = primary_dir == secondary_dir;

        if both_directional && same_direction {
            // Both sources agree directionally → high validation confidence
            let avg_score = (primary_score + secondary_score) / 2.0;
            let avg_weight = (primary_weight + secondary_weight) / 2.0;
            let factor = ValidatedFactor {
                name: format!("{} [validated]", factor_name),
                score: avg_score,
                direction: primary_dir,
                primary_source: primary_source.to_string(),
                cross_validated_by: vec![secondary_source.to_string()],
                validation_confidence: 0.85,
                weight: avg_weight,
                description: format!(
                    "{} ({:.2}) + {} ({:.2}) → both {} — high confidence",
                    primary_source,
                    primary_score,
                    secondary_source,
                    secondary_score,
                    if primary_dir == SkillDirection::Bullish {
                        "Bullish"
                    } else {
                        "Bearish"
                    }
                ),
            };
            (factor, None)
        } else if both_directional && !same_direction {
            // Sources disagree → conflict, penalize both
            let penalty = 0.3;
            let adjusted_primary = if primary_score > 0.5 {
                primary_score - penalty
            } else {
                primary_score + penalty
            };
            let adjusted_secondary = if secondary_score > 0.5 {
                secondary_score - penalty
            } else {
                secondary_score + penalty
            };
            let avg_score = (adjusted_primary + adjusted_secondary) / 2.0;
            let avg_weight = (primary_weight + secondary_weight) / 2.0;
            let conflict_record = CrossValidationConflict {
                factor_name: factor_name.to_string(),
                sources: vec![primary_source.to_string(), secondary_source.to_string()],
                primary_value: primary_score,
                conflicting_value: secondary_score,
                resolution: ConflictResolution::Penalized {
                    penalty,
                    adjusted_primary,
                    adjusted_conflicting: adjusted_secondary,
                },
            };
            let factor = ValidatedFactor {
                name: format!("{} [CONFLICT]", factor_name),
                score: avg_score,
                direction: SkillDirection::Neutral,
                primary_source: primary_source.to_string(),
                cross_validated_by: vec![secondary_source.to_string()],
                validation_confidence: 0.30,
                weight: avg_weight * 0.5,
                description: format!(
                    "CONFLICT: {} ({:.2}) says {} but {} ({:.2}) says {} — reduced confidence",
                    primary_source,
                    primary_score,
                    if primary_dir == SkillDirection::Bullish {
                        "Bullish"
                    } else {
                        "Bearish"
                    },
                    secondary_source,
                    secondary_score,
                    if secondary_dir == SkillDirection::Bullish {
                        "Bullish"
                    } else {
                        "Bearish"
                    },
                ),
            };
            (factor, Some(conflict_record))
        } else {
            // At least one is neutral → moderate validation
            let avg_score = if primary_dir == SkillDirection::Neutral {
                secondary_score
            } else if secondary_dir == SkillDirection::Neutral {
                primary_score
            } else {
                (primary_score + secondary_score) / 2.0
            };
            let dominant_dir = if primary_dir != SkillDirection::Neutral {
                primary_dir
            } else if secondary_dir != SkillDirection::Neutral {
                secondary_dir
            } else {
                SkillDirection::Neutral
            };
            let factor = ValidatedFactor {
                name: format!("{} [partial]", factor_name),
                score: avg_score,
                direction: dominant_dir,
                primary_source: primary_source.to_string(),
                cross_validated_by: vec![secondary_source.to_string()],
                validation_confidence: 0.60,
                weight: (primary_weight + secondary_weight) / 2.0,
                description: format!(
                    "{} ({:.2}) + {} ({:.2}) — one neutral, moderate confidence",
                    primary_source, primary_score, secondary_source, secondary_score
                ),
            };
            (factor, None)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SUPERINTELLIGENCE LAYER — orchestrates cross-validation + conviction + trace
// ═══════════════════════════════════════════════════════════════════════════════

/// The SuperIntelligence layer wraps cross-validation, conviction stacking, and
/// decision traceability into a single call for the StrategyDecisionAgent.
///
/// Usage:
///   let result = SuperIntelligence::analyze(
///       &state, symbol, current_price,
///       &aggregated_signal, &evidence,
///       proposed_action, proposed_confidence,
///     ).await;
///   // result.decision_trace.format_for_log() → ranked factors
///   // result.conviction.final_conviction → single number for downstream
///   // result.validation.summary → cross-validation results
pub struct SuperIntelligence;

#[derive(Debug, Clone)]
pub struct SuperIntelligenceResult {
    pub conviction: ConvictionStack,
    pub validation: CrossValidationReport,
    pub memory: MemoryContext,
    pub decision_trace: DecisionTrace,
    /// The adjusted action after considering all factors
    pub recommended_action: String,
    /// The adjusted confidence
    pub recommended_confidence: f64,
    /// Whether the recommendation should be followed
    pub should_proceed: bool,
}

impl SuperIntelligence {
    /// Run the full SuperIntelligence analysis pipeline.
    ///
    /// 1. Gather memory context (win rate from similar episodes)
    /// 2. Run cross-validation on all skill results
    /// 3. Compute conviction stack (multi-factor)
    /// 4. Build decision trace (ranked factors)
    /// 5. Adjust final action + confidence
    pub async fn analyze(
        state: &SharedState,
        symbol: &str,
        current_price: f64,
        aggregated_signal: &AggregatedSignal,
        evidence: &EvidenceBuilder,
        proposed_action: &str,
        proposed_confidence: f64,
    ) -> SuperIntelligenceResult {
        // 1. Memory context
        let memory = MemoryContext::from_vector_memory(state, symbol, current_price).await;

        // 2. Build skill results map from aggregated signal + evidence
        let mut skill_results: HashMap<String, (f64, SkillDirection, f64)> = HashMap::new();
        // Extract from evidence
        for ev in &evidence.evidences {
            // Infer direction from score
            let dir = if ev.score > 0.1 {
                SkillDirection::Bullish
            } else if ev.score < -0.1 {
                SkillDirection::Bearish
            } else {
                SkillDirection::Neutral
            };
            // Score in [0,1] range from [-1,1] evidence score
            let score = ((ev.score + 1.0) / 2.0).clamp(0.0, 1.0);
            skill_results.insert(ev.factor.clone(), (score, dir, ev.weight));
        }

        // 3. Cross-validation
        let validation = CrossValidationEngine::validate(&skill_results);

        // 4. Regime
        let regime = *state.market_regime.read().await;
        let regime_label = match &regime {
            Some(r) => format!("{:?}", r),
            None => "Ranging".to_string(),
        };
        let regime_enum = regime.unwrap_or(MarketRegime::Ranging);

        // 5. Portfolio heat
        let portfolio_heat = {
            let p = state.portfolio.read().await;
            if p.total_equity > 0.0 {
                p.open_positions
                    .iter()
                    .map(|pos| pos.risk_amount)
                    .sum::<f64>()
                    / p.total_equity
            } else {
                0.0
            }
        };

        // 6. Pattern strength from state (from candlestick pattern detection)
        let pattern_strength = {
            let pats = state.last_patterns.read().await;
            pats.get(symbol).map(|p| {
                if p.is_empty() {
                    0.5
                } else {
                    // Calculate average pattern direction alignment
                    let bullish_count =
                        p.iter().filter(|pat| pat.direction == "bullish").count() as f64;
                    let bearish_count =
                        p.iter().filter(|pat| pat.direction == "bearish").count() as f64;
                    let total = p.len() as f64;
                    // Map to 0-1: bullish dominant → high, bearish dominant → low, mixed → 0.5
                    let net = (bullish_count - bearish_count) / total.max(1.0);
                    ((net + 1.0) / 2.0).clamp(0.0, 1.0)
                }
            })
        };

        // 7. Multi-TF confirmation from state
        let multi_tf_confirmation: Option<f64> = {
            let mtf = state.last_mtf_patterns.read().await;
            mtf.get(symbol).map(|m| {
                let bullish_score: f64 = match m.bullish_confirmation {
                    ConfirmationLevel::Strong => 1.0,
                    ConfirmationLevel::Moderate => 0.7,
                    ConfirmationLevel::Weak => 0.4,
                    ConfirmationLevel::None => 0.0,
                };
                let bearish_score: f64 = match m.bearish_confirmation {
                    ConfirmationLevel::Strong => 1.0,
                    ConfirmationLevel::Moderate => 0.7,
                    ConfirmationLevel::Weak => 0.4,
                    ConfirmationLevel::None => 0.0,
                };
                bullish_score.max(bearish_score)
            })
        };

        // 8. Cross-validation score as synthesis
        let cross_val_score = Some(validation.overall_validation_score);

        // 9. Conviction stack (extended 8-factor)
        let conviction = ConvictionStack::compute_extended(
            aggregated_signal,
            portfolio_heat,
            memory.win_rate,
            &regime_enum,
            pattern_strength,
            multi_tf_confirmation,
            cross_val_score,
        );

        // 7. Regime threshold
        let regime_threshold = match &regime {
            Some(MarketRegime::TrendingBull) => 0.50,
            Some(MarketRegime::TrendingBear) => 0.80,
            Some(MarketRegime::Ranging) => 0.50,
            Some(MarketRegime::Volatile) => 0.75,
            Some(MarketRegime::LowLiquidity) => 0.50,
            None => 0.50,
        };

        // 8. Decision trace
        let decision_trace = DecisionTrace::new(
            proposed_action,
            proposed_confidence,
            &conviction,
            &validation,
            &regime_label,
            regime_threshold,
            portfolio_heat,
        );

        // 9. Adjust action + confidence
        // The SuperIntelligence can upgrade HOLD to BUY or downgrade BUY to HOLD
        // based on the full conviction stack + cross-validation
        let should_buy = conviction.final_conviction >= regime_threshold
            && validation.overall_validation_score >= 0.4;

        let recommended_action = if proposed_action == "BUY" && should_buy {
            "BUY"
        } else if proposed_action == "HOLD" && should_buy && conviction.final_conviction >= 0.60 {
            // Upgrade HOLD to BUY if conviction is very strong
            "BUY"
        } else {
            "HOLD"
        };

        let recommended_confidence = if recommended_action == "BUY" {
            // Conviction-stacked confidence
            (conviction.final_conviction * 0.7 + validation.overall_validation_score * 0.3)
                .clamp(0.0, 0.95)
        } else {
            proposed_confidence * 0.5 // Reduce confidence for HOLD
        };

        let should_proceed = recommended_action == "BUY" && recommended_confidence >= 0.45;

        println!(
            "[SuperIntelligence] 🔬 {} → {} (conf {:.1}%) | conviction={:.1}% validation={:.1}% memory={}ep | {}",
            proposed_action,
            recommended_action,
            recommended_confidence * 100.0,
            conviction.final_conviction * 100.0,
            validation.overall_validation_score * 100.0,
            memory.similar_episodes_count,
            if should_proceed { "✅ PROCEED" } else { "❌ BLOCKED" }
        );

        // Print the decision trace for logging
        println!("{}", decision_trace.format_for_log());

        SuperIntelligenceResult {
            conviction,
            validation,
            memory,
            decision_trace,
            recommended_action: recommended_action.to_string(),
            recommended_confidence,
            should_proceed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tredo_core::AggregatedSignal;

    fn make_signal(
        net: f64,
        conviction: f64,
        bull: usize,
        bear: usize,
        neut: usize,
    ) -> AggregatedSignal {
        AggregatedSignal {
            net_signal: net,
            bullish_strength: if net > 0.0 { net * conviction } else { 0.0 },
            bearish_strength: if net < 0.0 { (-net) * conviction } else { 0.0 },
            conviction,
            consensus: if bull > bear {
                Some(SkillDirection::Bullish)
            } else if bear > bull {
                Some(SkillDirection::Bearish)
            } else {
                None
            },
            participating_count: bull + bear,
            bullish_count: bull,
            bearish_count: bear,
            neutral_count: neut,
        }
    }

    #[test]
    fn test_conviction_stack_trending_bull() {
        // Strong bullish signal in TrendingBull regime
        let signal = make_signal(0.6, 0.7, 4, 1, 3);
        let stack =
            ConvictionStack::compute(&signal, 0.02, Some(0.65), &MarketRegime::TrendingBull);

        // Directional should dominate (w=0.30 in 8-factor)
        let dir_factor = stack
            .factors
            .iter()
            .find(|f| f.name == "Directional")
            .unwrap();
        assert!((dir_factor.weight_in_decision - 0.30).abs() < 0.01);
        assert!(stack.final_conviction > 0.5);
    }

    #[test]
    fn test_conviction_stack_low_liquidity() {
        // LowLiquidity regime → risk dominates
        let signal = make_signal(0.3, 0.4, 2, 1, 5);
        let stack = ConvictionStack::compute(&signal, 0.15, Some(0.4), &MarketRegime::LowLiquidity);

        // Risk should have highest weight (0.35 in 8-factor)
        let risk_factor = stack.factors.iter().find(|f| f.name == "Risk").unwrap();
        assert!((risk_factor.weight_in_decision - 0.35).abs() < 0.01);
        // High heat (0.15 → risk_conviction = 1.0 - 0.75 = 0.25) should pull conviction down
        assert!(stack.final_conviction < 0.5);
    }

    #[test]
    fn test_conviction_stack_volatile() {
        // Volatile regime → risk + memory matter most
        let signal = make_signal(0.4, 0.5, 3, 2, 3);
        let stack = ConvictionStack::compute(&signal, 0.06, Some(0.3), &MarketRegime::Volatile);

        // Risk weight = 0.25, Memory weight = 0.18 (8-factor)
        let risk_factor = stack.factors.iter().find(|f| f.name == "Risk").unwrap();
        let mem_factor = stack.factors.iter().find(|f| f.name == "Memory").unwrap();
        assert!((risk_factor.weight_in_decision - 0.25).abs() < 0.01);
        assert!((mem_factor.weight_in_decision - 0.18).abs() < 0.01);
        // Low memory win rate (0.3) should reduce conviction
        assert!(stack.memory_conviction < 0.5);
    }

    #[test]
    fn test_cross_validation_both_bullish() {
        let mut results = HashMap::new();
        results.insert(
            "MarketMetricsMeter".to_string(),
            (0.75, SkillDirection::Bullish, 0.29),
        );
        results.insert(
            "RegimeDetector".to_string(),
            (0.80, SkillDirection::Bullish, 0.10),
        );
        results.insert(
            "OrderFlow".to_string(),
            (0.65, SkillDirection::Bullish, 0.06),
        );
        results.insert(
            "FundingRate".to_string(),
            (0.40, SkillDirection::Bullish, 0.03),
        );

        let _signal = make_signal(0.5, 0.6, 3, 0, 1);
        let report = CrossValidationEngine::validate(&results);

        // Should have validated pairs (MMM↔Regime, OF↔FR)
        assert!(report
            .factors
            .iter()
            .any(|f| f.name.contains("[validated]")));
        // Should have NO conflicts (all agree)
        assert!(report.conflicts.is_empty());
    }

    #[test]
    fn test_cross_validation_conflict() {
        let mut results = HashMap::new();
        results.insert(
            "MarketMetricsMeter".to_string(),
            (0.75, SkillDirection::Bullish, 0.29),
        );
        results.insert(
            "RegimeDetector".to_string(),
            (0.30, SkillDirection::Bearish, 0.10),
        );
        results.insert(
            "OrderFlow".to_string(),
            (0.65, SkillDirection::Bullish, 0.06),
        );
        results.insert(
            "FundingRate".to_string(),
            (0.35, SkillDirection::Bearish, 0.03),
        );

        let _signal = make_signal(0.2, 0.4, 2, 2, 0);
        let report = CrossValidationEngine::validate(&results);

        // Should have at least one CONFLICT
        assert!(report.factors.iter().any(|f| f.name.contains("[CONFLICT]")));
        assert!(!report.conflicts.is_empty());
        // Validation score should be penalized
        assert!(report.overall_validation_score < 0.7);
    }

    #[test]
    fn test_decision_trace_ranking() {
        let signal = make_signal(0.5, 0.7, 4, 1, 3);
        let conv = ConvictionStack::compute(&signal, 0.03, Some(0.7), &MarketRegime::TrendingBull);
        let report = CrossValidationReport::empty();
        let trace = DecisionTrace::new("BUY", 0.75, &conv, &report, "TrendingBull", 0.50, 0.03);

        assert_eq!(trace.final_action, "BUY");
        assert_eq!(trace.regime, "TrendingBull");
        assert!(trace.final_conviction > 0.0);
    }
}
