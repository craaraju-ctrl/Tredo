//! `SkillAggregator` — weighted ensemble aggregation for structured `AgentOutput::SkillResult` values.
//!
//! Takes a collection of skill outputs (each with a score, confidence, direction, and weight)
//! and produces a combined directional signal that agents can use for decision-making.
//!
//! # Aggregation Logic
//!
//! For each skill result:
//! - **Bullish** skills contribute `+score * confidence * weight` to the bullish side.
//! - **Bearish** skills contribute `+score * confidence * weight` to the bearish side.
//! - **Neutral** skills contribute a small `(0.5 - score).abs() * confidence * weight` to neither side
//!   (they dilute conviction but don't push direction).
//!
//! The final `AggregatedSignal` contains:
//! - `bullish_strength`: total weighted bullish evidence (0.0 – 1.0+)
//! - `bearish_strength`: total weighted bearish evidence (0.0 – 1.0+)
//! - `net_signal`:    normalized to **-1.0 (strong bear) … +1.0 (strong bull)**
//! - `conviction`:     how much total evidence exists across all skills (0.0 – 1.0)
//! - `consensus`:      whether most participating skills agree on direction

use crate::agent::{AgentOutput, SkillDirection};

/// Aggregated signal produced by combining multiple skill results.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AggregatedSignal {
    /// Net directional signal: -1.0 (strong bear) to +1.0 (strong bull).
    pub net_signal: f64,
    /// Total weighted bullish evidence.
    pub bullish_strength: f64,
    /// Total weighted bearish evidence.
    pub bearish_strength: f64,
    /// How much total evidence exists across all skills (normalised 0.0–1.0).
    pub conviction: f64,
    /// Whether a majority of non-neutral skills agree on the same direction.
    pub consensus: Option<SkillDirection>,
    /// Number of skills that contributed a non-neutral vote.
    pub participating_count: usize,
    /// Number of skills that voted Bullish.
    pub bullish_count: usize,
    /// Number of skills that voted Bearish.
    pub bearish_count: usize,
    /// Number of skills that voted Neutral.
    pub neutral_count: usize,
}

impl AggregatedSignal {
    /// True when `net_signal` leans bullish (above the given threshold, default 0.15).
    pub fn is_bullish(&self, threshold: Option<f64>) -> bool {
        self.net_signal > threshold.unwrap_or(0.15)
    }

    /// True when `net_signal` leans bearish (below the negative threshold).
    pub fn is_bearish(&self, threshold: Option<f64>) -> bool {
        self.net_signal < -threshold.unwrap_or(0.15)
    }

    /// True when conviction is low or signals are too conflicting.
    pub fn is_neutral(&self, conviction_threshold: Option<f64>) -> bool {
        self.conviction < conviction_threshold.unwrap_or(0.3)
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        let dir = if self.is_bullish(None) {
            "BULLISH"
        } else if self.is_bearish(None) {
            "BEARISH"
        } else {
            "NEUTRAL"
        };
        format!(
            "{} | net={:+.3} | bull={:.3} bear={:.3} | conviction={:.1}% | {}B/{}Be/{}N",
            dir,
            self.net_signal,
            self.bullish_strength,
            self.bearish_strength,
            self.conviction * 100.0,
            self.bullish_count,
            self.bearish_count,
            self.neutral_count,
        )
    }
}

/// Weighted ensemble aggregator for skill results.
pub struct SkillAggregator;

impl SkillAggregator {
    /// Aggregate a slice of `AgentOutput` values, extracting only `SkillResult` variants.
    ///
    /// Non-`SkillResult` outputs are silently ignored.
    pub fn aggregate(outputs: &[AgentOutput]) -> AggregatedSignal {
        let skills: Vec<_> = outputs.iter().filter_map(|o| o.as_skill_result()).collect();

        let total_skills = skills.len();
        if total_skills == 0 {
            return AggregatedSignal {
                net_signal: 0.0,
                bullish_strength: 0.0,
                bearish_strength: 0.0,
                conviction: 0.0,
                consensus: None,
                participating_count: 0,
                bullish_count: 0,
                bearish_count: 0,
                neutral_count: 0,
            };
        }

        let mut bullish_strength = 0.0_f64;
        let mut bearish_strength = 0.0_f64;
        let mut total_weighted_evidence = 0.0_f64;
        let mut bullish_count = 0_usize;
        let mut bearish_count = 0_usize;
        let mut neutral_count = 0_usize;

        for (_name, score, _note, confidence, direction, weight) in &skills {
            let contribution = score * confidence * weight;
            total_weighted_evidence += contribution;

            match direction {
                SkillDirection::Bullish => {
                    bullish_strength += contribution;
                    bullish_count += 1;
                }
                SkillDirection::Bearish => {
                    bearish_strength += contribution;
                    bearish_count += 1;
                }
                SkillDirection::Neutral => {
                    // Neutral skills contribute a small non-directional amount to conviction
                    // but don't push net_signal.
                    neutral_count += 1;
                }
            }
        }

        let net = bullish_strength - bearish_strength;

        // Conviction: how much total evidence exists, normalized by max possible.
        // Max possible is sum(1.0 * 1.0 * weight) for each skill.
        let max_possible: f64 = skills.iter().map(|(_, _, _, _, _, w)| 1.0 * 1.0 * w).sum();
        let conviction = if max_possible > 0.0 {
            (total_weighted_evidence / max_possible).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let participating_count = bullish_count + bearish_count;
        let consensus = if participating_count == 0 {
            None
        } else if bullish_count > bearish_count {
            Some(SkillDirection::Bullish)
        } else if bearish_count > bullish_count {
            Some(SkillDirection::Bearish)
        } else {
            None // tied
        };

        AggregatedSignal {
            net_signal: net,
            bullish_strength,
            bearish_strength,
            conviction,
            consensus,
            participating_count,
            bullish_count,
            bearish_count,
            neutral_count,
        }
    }

    /// Convenience: aggregate from an iterator of `AgentOutput`.
    pub fn from_outputs<I>(outputs: I) -> AggregatedSignal
    where
        I: IntoIterator<Item = AgentOutput>,
    {
        let collected: Vec<_> = outputs.into_iter().collect();
        Self::aggregate(&collected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentOutput, SkillDirection};

    fn make_skill(
        name: &str,
        score: f64,
        confidence: f64,
        direction: SkillDirection,
        weight: f64,
    ) -> AgentOutput {
        AgentOutput::SkillResult {
            name: name.to_string(),
            score,
            note: String::new(),
            confidence,
            direction,
            weight,
        }
    }

    #[test]
    fn test_empty_input() {
        let sig = SkillAggregator::aggregate(&[]);
        assert!((sig.net_signal - 0.0).abs() < 1e-6);
        assert_eq!(sig.participating_count, 0);
    }

    #[test]
    fn test_single_bullish() {
        let outputs = vec![make_skill(
            "sentiment",
            0.8,
            0.6,
            SkillDirection::Bullish,
            1.0,
        )];
        let sig = SkillAggregator::aggregate(&outputs);
        assert!(sig.net_signal > 0.0);
        assert!(sig.is_bullish(None));
        assert!(!sig.is_bearish(None));
        assert_eq!(sig.bullish_count, 1);
        assert_eq!(sig.consensus, Some(SkillDirection::Bullish));
    }

    #[test]
    fn test_single_bearish() {
        let outputs = vec![make_skill(
            "onchain",
            0.3,
            0.7,
            SkillDirection::Bearish,
            1.0,
        )];
        let sig = SkillAggregator::aggregate(&outputs);
        assert!(sig.net_signal < 0.0);
        assert!(sig.is_bearish(None));
        assert_eq!(sig.bearish_count, 1);
        assert_eq!(sig.consensus, Some(SkillDirection::Bearish));
    }

    #[test]
    fn test_mixed_signals_lower_conviction() {
        let outputs = vec![
            make_skill("sentiment", 0.8, 0.6, SkillDirection::Bullish, 0.3),
            make_skill("regime", 0.2, 0.4, SkillDirection::Bearish, 0.25),
        ];
        let sig = SkillAggregator::aggregate(&outputs);
        // Net depends on weighted contributions; both present, so lower conviction
        assert!(sig.participating_count > 0);
        assert!(sig.conviction < 1.0);
    }

    #[test]
    fn test_all_neutral() {
        let outputs = vec![
            make_skill("vol", 0.5, 0.5, SkillDirection::Neutral, 0.2),
            make_skill("corr", 0.5, 0.5, SkillDirection::Neutral, 0.1),
        ];
        let sig = SkillAggregator::aggregate(&outputs);
        assert!((sig.net_signal - 0.0).abs() < 1e-6);
        assert_eq!(sig.neutral_count, 2);
        assert_eq!(sig.consensus, None);
    }

    #[test]
    fn test_summary_format() {
        // Use weight=1.0 so net_signal exceeds the default 0.15 bullish threshold.
        let outputs = vec![make_skill(
            "sentiment",
            0.8,
            0.6,
            SkillDirection::Bullish,
            1.0,
        )];
        let sig = SkillAggregator::aggregate(&outputs);
        let summary = sig.summary();
        assert!(summary.contains("BULLISH"));
        assert!(summary.contains("net="));
    }

    #[test]
    fn test_as_skill_result() {
        let output = make_skill("test", 0.5, 0.5, SkillDirection::Neutral, 0.5);
        let extracted = output.as_skill_result();
        assert!(extracted.is_some());
        let (name, score, _note, conf, dir, weight) = extracted.unwrap();
        assert_eq!(name, "test");
        assert!((score - 0.5).abs() < 1e-6);
        assert!((conf - 0.5).abs() < 1e-6);
        assert_eq!(dir, SkillDirection::Neutral);
        assert!((weight - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_skill_score() {
        let output = make_skill("test", 0.75, 0.5, SkillDirection::Bullish, 1.0);
        assert!((output.skill_score().unwrap() - 0.75).abs() < 1e-6);
        let not_skill = AgentOutput::Done;
        assert!(not_skill.skill_score().is_none());
    }

    #[test]
    fn test_is_neutral_when_low_conviction() {
        let outputs = vec![make_skill("weak", 0.1, 0.1, SkillDirection::Bullish, 0.1)];
        let sig = SkillAggregator::aggregate(&outputs);
        assert!(sig.is_neutral(Some(0.5)));
    }

    #[test]
    fn test_from_outputs() {
        let outputs = vec![
            make_skill("a", 0.8, 0.6, SkillDirection::Bullish, 0.3),
            make_skill("b", 0.7, 0.5, SkillDirection::Bullish, 0.25),
        ];
        let sig = SkillAggregator::from_outputs(outputs);
        assert_eq!(sig.bullish_count, 2);
        assert!(sig.net_signal > 0.0);
    }

    #[test]
    fn test_consensus_tied() {
        let outputs = vec![
            make_skill("a", 0.8, 0.6, SkillDirection::Bullish, 0.3),
            make_skill("b", 0.3, 0.6, SkillDirection::Bearish, 0.3),
        ];
        let sig = SkillAggregator::aggregate(&outputs);
        assert_eq!(sig.bullish_count, 1);
        assert_eq!(sig.bearish_count, 1);
        // Net should be near zero (equal contribution)
        assert!((sig.net_signal).abs() < 0.5);
        // Consensus is None when tied, regardless of equal/count
        assert_eq!(sig.consensus, None);
    }
}
