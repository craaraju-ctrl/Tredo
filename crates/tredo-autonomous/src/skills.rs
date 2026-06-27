use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export SkillResult from walk_forward_runner to avoid duplicate definitions.
pub use crate::walk_forward_runner::SkillResult;

/// Aggregated cognitive output from multiple skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfluenceScorer {
    pub net_score: f64,
    pub conviction: f64,
    pub skill_count: usize,
}

impl ConfluenceScorer {
    /// Aggregate multiple SkillResult values into a single weighted consensus.
    /// Uses confidence-weighted averaging.
    pub fn aggregate(skills: Vec<SkillResult>, weights: &HashMap<String, f64>) -> Self {
        if skills.is_empty() {
            return Self {
                net_score: 0.0,
                conviction: 0.0,
                skill_count: 0,
            };
        }

        let total_weight: f64 = weights.values().sum::<f64>().max(1.0);
        let weighted_sum: f64 = skills
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let w = weights
                    .get(&format!("skill_{}", i))
                    .copied()
                    .unwrap_or(1.0 / skills.len() as f64);
                s.score * w * s.confidence
            })
            .sum();

        let avg_confidence: f64 =
            skills.iter().map(|s| s.confidence).sum::<f64>() / skills.len() as f64;

        Self {
            net_score: (weighted_sum / total_weight).clamp(-1.0, 1.0),
            conviction: avg_confidence.clamp(0.0, 1.0),
            skill_count: skills.len(),
        }
    }

    /// Returns true if the aggregated signal is bullish above the given threshold.
    pub fn is_bullish(&self, threshold: Option<f64>) -> bool {
        self.net_score > threshold.unwrap_or(0.2)
    }

    /// Returns true if the aggregated signal is bearish below the given threshold.
    pub fn is_bearish(&self, threshold: Option<f64>) -> bool {
        self.net_score < threshold.unwrap_or(-0.2)
    }
}
