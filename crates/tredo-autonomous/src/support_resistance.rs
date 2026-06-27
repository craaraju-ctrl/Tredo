// SupportResistanceSkill — detects key support/resistance zones from price action.
// Implements AgentSkill for pluggability into the debate layer and confluence scorer.
// No LLM required — pure deterministic analysis.
//
// UPGRADE: Now directional — price near support + rising = Bullish (bounce).
//   Price near resistance + falling = Bearish (rejection).
//   Otherwise = Neutral.

use crate::helpers::compute_support_resistance;
use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput};

pub struct SupportResistanceSkill {
    pub state: SharedState,
}

impl SupportResistanceSkill {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn analyze(&self, symbol: &str) -> (Vec<f64>, Vec<f64>, f64, f64) {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if bars.len() >= 30 {
                let (supports, resistances) = compute_support_resistance(bars, 30);
                let current_price = bars.last().unwrap().close;
                let n = bars.len();
                let price_change = if n >= 4 {
                    (bars[n - 1].close - bars[n - 4].close) / bars[n - 4].close
                } else {
                    0.0
                };

                let nearest_s = supports
                    .iter()
                    .map(|&s| (s - current_price).abs())
                    .fold(f64::MAX, f64::min);
                let nearest_r = resistances
                    .iter()
                    .map(|&r| (r - current_price).abs())
                    .fold(f64::MAX, f64::min);
                let proximity = if nearest_s < nearest_r {
                    nearest_s / current_price
                } else {
                    nearest_r / current_price
                };
                return (supports, resistances, proximity, price_change);
            }
        }
        (vec![], vec![], 0.05, 0.0)
    }
}

#[async_trait]
impl AgentSkill for SupportResistanceSkill {
    fn name(&self) -> &str {
        "SupportResistance"
    }
    fn description(&self) -> &str {
        "Detects key support/resistance zones from swing highs/lows. Now directional: price near support + rising = Bullish (bounce), near resistance + falling = Bearish (rejection)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (supports, resistances, proximity, price_change) =
                self.analyze(&context.symbol).await;
            let note = format!(
                "S={:?} R={:?} proximity={:.4} chg={:+.4}",
                supports
                    .iter()
                    .take(3)
                    .map(|s| format!("{:.2}", s))
                    .collect::<Vec<_>>(),
                resistances
                    .iter()
                    .take(3)
                    .map(|r| format!("{:.2}", r))
                    .collect::<Vec<_>>(),
                proximity,
                price_change
            );

            // Directional logic: S/R proximity + price direction
            // Find which is nearest: support or resistance
            let nearest_s = supports
                .iter()
                .map(|&s| (s - context.current_price).abs())
                .fold(f64::MAX, f64::min);
            let nearest_r = resistances
                .iter()
                .map(|&r| (r - context.current_price).abs())
                .fold(f64::MAX, f64::min);

            let (direction, score) =
                if nearest_s < nearest_r && proximity < 0.02 && price_change > 0.005 {
                    // Price near support + bouncing up = Bullish
                    (tredo_core::agent::SkillDirection::Bullish, 0.70)
                } else if nearest_r < nearest_s && proximity < 0.02 && price_change < -0.005 {
                    // Price near resistance + rejecting down = Bearish
                    (tredo_core::agent::SkillDirection::Bearish, 0.30)
                } else {
                    // No clear S/R directional signal
                    (
                        tredo_core::agent::SkillDirection::Neutral,
                        1.0 - proximity.clamp(0.0, 1.0),
                    )
                };

            println!(
                "[Skill] {} for {}: {}",
                self.name(),
                context.symbol,
                &note[..note.len().min(120)]
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note,
                confidence: if proximity < 0.02 { 0.85 } else { 0.6 },
                direction,
                weight: 0.0, // Weight from DisciplineRules (single source of truth)
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
