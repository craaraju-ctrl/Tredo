// OrderFlowSkill — buy/sell pressure imbalance analyzer.
// Implements AgentSkill. No LLM. Pure deterministic volume-price analysis.

use crate::helpers::compute_order_flow_imbalance;
use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput};

pub struct OrderFlowSkill {
    pub state: SharedState,
}

impl OrderFlowSkill {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn analyze(&self, symbol: &str) -> (f64, String) {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if bars.len() >= 20 {
                let imbalance = compute_order_flow_imbalance(bars, 20);
                let strength = if imbalance.abs() > 0.6 {
                    "strong"
                } else if imbalance.abs() > 0.3 {
                    "moderate"
                } else {
                    "weak"
                };
                return (imbalance, strength.to_string());
            }
        }
        (0.0, "neutral".to_string())
    }
}

#[async_trait]
impl AgentSkill for OrderFlowSkill {
    fn name(&self) -> &str {
        "OrderFlow"
    }
    fn description(&self) -> &str {
        "Analyzes buy/sell pressure imbalance from bar close position and volume. Strong positive = aggressive buying, negative = aggressive selling."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (imbalance, strength) = self.analyze(&context.symbol).await;
            let note = format!("imbalance={:.2} strength={}", imbalance, strength);
            println!("[Skill] {} for {}: {}", self.name(), context.symbol, note);
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score: (imbalance + 1.0) / 2.0, // map [-1, 1] to [0, 1]
                note,
                confidence: if imbalance.abs() > 0.5 { 0.8 } else { 0.55 },
                direction: if imbalance > 0.2 {
                    tredo_core::agent::SkillDirection::Bullish
                } else if imbalance < -0.2 {
                    tredo_core::agent::SkillDirection::Bearish
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
