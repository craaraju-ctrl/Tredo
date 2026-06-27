// LiquiditySkill — market liquidity and slippage risk analyzer.
// Implements AgentSkill. No LLM. Pure deterministic analysis from bar data.
//
// UPGRADE: Now directional — high depth = Bullish (easy execution, tight fills).
//   Low depth = Bearish (slippage risk, wide spreads).
//   Medium depth = Neutral.

use crate::helpers::{compute_liquidity, LiquiditySnapshot};
use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput};

pub struct LiquiditySkill {
    pub state: SharedState,
}

impl LiquiditySkill {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn analyze(&self, symbol: &str) -> LiquiditySnapshot {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if let Some(last) = bars.last() {
                return compute_liquidity(bars, last.close);
            }
        }
        LiquiditySnapshot {
            spread_pct: 0.001,
            depth_score: 0.5,
            slippage_risk: 0.002,
            market_quality: "fair".to_string(),
        }
    }
}

#[async_trait]
impl AgentSkill for LiquiditySkill {
    fn name(&self) -> &str {
        "Liquidity"
    }
    fn description(&self) -> &str {
        "Analyzes market liquidity quality: spread, depth, slippage risk. Now directional: high depth = Bullish (easy execution), low depth = Bearish (slippage risk)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let snap = self.analyze(&context.symbol).await;
            let note = format!(
                "spread={:.4}% depth={:.2} slippage={:.4}% quality={}",
                snap.spread_pct * 100.0,
                snap.depth_score,
                snap.slippage_risk * 100.0,
                snap.market_quality
            );

            // Directional logic: liquidity quality affects execution confidence
            let (direction, score) = if snap.depth_score > 0.7 {
                // High liquidity = easy execution = Bullish
                (tredo_core::agent::SkillDirection::Bullish, snap.depth_score)
            } else if snap.depth_score < 0.3 {
                // Poor liquidity = slippage risk = Bearish
                (tredo_core::agent::SkillDirection::Bearish, snap.depth_score)
            } else {
                // Adequate liquidity = Neutral
                (tredo_core::agent::SkillDirection::Neutral, snap.depth_score)
            };

            println!(
                "[Skill] {} for {}: {} dir={:?}",
                self.name(),
                context.symbol,
                note,
                direction
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note,
                confidence: if snap.depth_score > 0.7 { 0.8 } else { 0.55 },
                direction,
                weight: 0.0, // Weight from DisciplineRules (single source of truth)
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
