// VolatilityCalculator helper (new tool/skill)
// Computes ATR, expansion for breakout/vol strategies.
// Upgrades risk and MI with regime-aware vol detection.
// Note: Pure helper (methods only), not full Agent impl, to match core AgentInput enum.
//
// UPGRADE: Now directional — vol expansion + price direction = breakout signal.
//   - Vol expansion (>1.5%) + price rising = Bullish (breakout volatility)
//   - Vol expansion (>1.5%) + price falling = Bearish (panic sell-off)
//   - Low vol = Neutral (no clear directional signal)

use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput};

pub struct VolatilityCalculator {
    pub state: SharedState,
}

impl VolatilityCalculator {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn compute_volatility(&self, symbol: &str, price: f64) -> (f64, bool) {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if bars.len() < 15 {
                return (0.01, false);
            }
            let mut atr = 0.0;
            for bar in bars.iter().skip(bars.len() - 14) {
                let range = (bar.high - bar.low).abs();
                atr += range;
            }
            atr /= 14.0;
            let atr_pct = atr / price;
            let expansion = atr_pct > 0.015; // vol expansion threshold
            return (atr_pct, expansion);
        }
        (0.01, false)
    }

    /// Check price direction over last 3 bars to determine breakout vs panic
    pub async fn price_direction(&self, symbol: &str) -> f64 {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if bars.len() >= 4 {
                let n = bars.len();
                let change = (bars[n - 1].close - bars[n - 4].close) / bars[n - 4].close;
                return change;
            }
        }
        0.0
    }
}

#[async_trait]
impl AgentSkill for VolatilityCalculator {
    fn name(&self) -> &str {
        "VolatilityCalculator"
    }
    fn description(&self) -> &str {
        "Computes ATR-based volatility % and expansion flag for a symbol/price. Now directional: expansion + rising price = Bullish (breakout), expansion + falling price = Bearish (panic)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (vol, exp) = self
                .compute_volatility(&context.symbol, context.current_price)
                .await;
            let price_change = self.price_direction(&context.symbol).await;

            // Directional logic: vol expansion + price direction = breakout/panic signal
            let (direction, score) = if exp && price_change > 0.01 {
                // Bullish breakout volatility
                (tredo_core::agent::SkillDirection::Bullish, 0.65)
            } else if exp && price_change < -0.01 {
                // Bearish panic selling
                (tredo_core::agent::SkillDirection::Bearish, 0.35)
            } else {
                // No directional vol signal
                (tredo_core::agent::SkillDirection::Neutral, vol)
            };

            println!(
                "[Skill] {} executed for {}: vol={:.4} expansion={} price_chg={:+.4} dir={:?}",
                self.name(),
                context.symbol,
                vol,
                exp,
                price_change,
                direction
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note: format!("expansion={} price_chg={:+.4}", exp, price_change),
                confidence: if exp { 0.8 } else { 0.5 },
                direction,
                weight: 0.2,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
