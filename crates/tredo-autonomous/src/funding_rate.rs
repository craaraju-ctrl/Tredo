// FundingRateSkill — crypto perpetual funding rate analysis proxy.
// Implements AgentSkill. No LLM. Uses local price-volatility proxy when real API unavailable.
// Positive funding = longs pay shorts (overly bullish). Negative = shorts pay longs (overly bearish).

use crate::helpers::compute_funding_rate_proxy;
use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput};

pub struct FundingRateSkill {
    pub state: SharedState,
}

impl FundingRateSkill {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn analyze(&self, symbol: &str) -> (f64, &'static str) {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if bars.len() >= 24 {
                return compute_funding_rate_proxy(bars, symbol);
            }
        }
        (0.0, "neutral")
    }
}

#[async_trait]
impl AgentSkill for FundingRateSkill {
    fn name(&self) -> &str {
        "FundingRate"
    }
    fn description(&self) -> &str {
        "Analyzes crypto perpetual funding rate proxy. Positive = crowded longs (caution), negative = crowded shorts (opportunity). Counter-sentiment indicator."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (funding, sentiment) = self.analyze(&context.symbol).await;
            let note = format!("funding={:.4}% sentiment={}", funding * 100.0, sentiment);
            println!("[Skill] {} for {}: {}", self.name(), context.symbol, note);
            // Funding is a COUNTER-indicator: positive = too many longs = bearish, negative = too many shorts = bullish
            let score = (-funding * 100.0 + 50.0).clamp(0.0, 100.0) / 100.0; // invert: -1% -> 1.0, +1% -> 0.0
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note,
                confidence: if funding.abs() > 0.005 { 0.75 } else { 0.5 },
                direction: if funding > 0.003 {
                    tredo_core::agent::SkillDirection::Bearish // crowded longs = bearish
                } else if funding < -0.003 {
                    tredo_core::agent::SkillDirection::Bullish // crowded shorts = bullish
                } else {
                    tredo_core::agent::SkillDirection::Neutral
                },
                weight: 0.0, // Weight from DisciplineRules (single source of truth)
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
