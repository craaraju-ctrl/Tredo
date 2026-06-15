use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillWeightSnapshot {
    pub episode_id: String,
    pub initial_weights: HashMap<String, f64>,
    pub updated_weights: HashMap<String, f64>,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct AttributionEngine {
    pub base_learning_rate: f64, // e.g., 0.05
    pub weight_floor: f64,       // 0.05 to preserve recovery paths
    pub weight_ceiling: f64,     // 0.40 to prevent single-skill dominance
}

impl AttributionEngine {
    pub fn new(base_learning_rate: f64) -> Self {
        Self {
            base_learning_rate,
            weight_floor: 0.05,
            weight_ceiling: 0.40,
        }
    }

    /// Performs a symmetric, normalized reward/penalty update on skill weights.
    /// This is the corrected self-evolution math that avoids the deflationary spiral.
    /// Both skill predictions and trade outcomes are mapped to the common [-1.0, +1.0] domain.
    pub fn tune_skill_weights(
        &self,
        episode_id: &str,
        actual_pnl_pct: f64,
        trade_direction: &str,                    // "BUY" or "SELL"
        skill_predictions: &HashMap<String, f64>, // Space: [-1.0, +1.0]
        current_weights: &HashMap<String, f64>,
        timestamp: u64,
    ) -> SkillWeightSnapshot {
        let mut updated_weights = HashMap::new();

        // Map trade outcome cleanly to the same [-1.0, +1.0] space as the skills
        let outcome_signal = match trade_direction {
            "BUY" => {
                if actual_pnl_pct > 0.0 {
                    1.0
                } else {
                    -1.0
                }
            }
            "SELL" => {
                if actual_pnl_pct > 0.0 {
                    -1.0
                } else {
                    1.0
                }
            }
            _ => 0.0,
        };

        for (skill_name, &old_weight) in current_weights {
            if let Some(&prediction) = skill_predictions.get(skill_name) {
                // Ensure prediction space matches outcome space bounds
                let clamped_pred = prediction.clamp(-1.0, 1.0);

                let is_correct = (clamped_pred >= 0.0 && outcome_signal >= 0.0)
                    || (clamped_pred < 0.0 && outcome_signal < 0.0);

                let delta = (clamped_pred - outcome_signal).abs(); // Domain: [0.0, 2.0]

                let new_weight = if is_correct {
                    let accuracy = (1.0 - (delta / 2.0)).max(0.0);
                    old_weight * (1.0 + (self.base_learning_rate * accuracy))
                } else {
                    let regret = (delta / 2.0).min(1.0);
                    old_weight * (1.0 - (self.base_learning_rate * regret))
                };

                // Apply structural floors and ceilings
                let bounded_weight = new_weight.clamp(self.weight_floor, self.weight_ceiling);
                updated_weights.insert(skill_name.clone(), bounded_weight);
            } else {
                updated_weights.insert(skill_name.clone(), old_weight);
            }
        }

        // Re-normalize weights so they sum precisely to 1.0
        let total_weight: f64 = updated_weights.values().sum();
        if total_weight > 0.0 {
            for val in updated_weights.values_mut() {
                *val /= total_weight;
            }
        }

        SkillWeightSnapshot {
            episode_id: episode_id.to_string(),
            initial_weights: current_weights.clone(),
            updated_weights,
            timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symmetric_update_reward() {
        let engine = AttributionEngine::new(0.1);
        let mut current = HashMap::new();
        current.insert("A".to_string(), 0.30);
        current.insert("B".to_string(), 0.30);
        let mut preds = HashMap::new();
        preds.insert("A".to_string(), 0.8);
        // Skill B didn't predict, keeps its weight

        let snap = engine.tune_skill_weights("ep1", 0.03, "BUY", &preds, &current, 123456);
        let new_a = *snap.updated_weights.get("A").unwrap();
        let new_b = *snap.updated_weights.get("B").unwrap();
        assert!(
            new_a > new_b,
            "Correct skill should get higher weight than skill that didn't predict"
        );
        let sum: f64 = snap.updated_weights.values().sum();
        assert!((sum - 1.0).abs() < 1e-9, "Weights must normalize to 1.0");
    }

    #[test]
    fn test_symmetric_update_penalty_and_normalize() {
        let engine = AttributionEngine::new(0.1);
        let mut current = HashMap::new();
        current.insert("A".to_string(), 0.3);
        current.insert("B".to_string(), 0.3);
        let mut preds = HashMap::new();
        preds.insert("A".to_string(), 0.9);

        let snap = engine.tune_skill_weights("ep2", -0.02, "BUY", &preds, &current, 123457);
        let sum: f64 = snap.updated_weights.values().sum();
        assert!((sum - 1.0).abs() < 1e-9);
        let new_a = *snap.updated_weights.get("A").unwrap();
        let new_b = *snap.updated_weights.get("B").unwrap();
        // A was wrong (predicted BUY, trade lost), B had no prediction
        // Both get clamped then normalized; A should be <= B
        assert!(
            new_a <= new_b,
            "Wrong skill should not exceed skill with no prediction"
        );
    }
}
