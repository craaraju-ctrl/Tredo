// VolatilityCalculator helper (new tool/skill)
// Computes ATR, expansion for breakout/vol strategies.
// Upgrades risk and MI with regime-aware vol detection.
// Note: Pure helper (methods only), not full Agent impl, to match core AgentInput enum.

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
}

#[async_trait]
impl AgentSkill for VolatilityCalculator {
    fn name(&self) -> &str {
        "VolatilityCalculator"
    }
    fn description(&self) -> &str {
        "Computes ATR-based volatility % and expansion flag for a symbol/price (how to quantify market volatility for risk and entry decisions)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (vol, exp) = self
                .compute_volatility(&context.symbol, context.current_price)
                .await;
            println!(
                "[Skill] {} executed for {}: vol={:.4} expansion={}",
                self.name(),
                context.symbol,
                vol,
                exp
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score: vol,
                note: format!("expansion={}", exp),
                confidence: if exp { 0.8 } else { 0.5 },
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
