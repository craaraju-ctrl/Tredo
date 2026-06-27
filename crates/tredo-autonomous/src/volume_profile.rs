// VolumeProfileSkill — computes Point of Control, Value Area, and volume distribution.
// Implements AgentSkill for pluggability. Pure deterministic analysis.

use crate::helpers::compute_volume_profile;
use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput};

pub struct VolumeProfileSkill {
    pub state: SharedState,
}

impl VolumeProfileSkill {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn analyze(&self, symbol: &str) -> (f64, f64, f64, f64) {
        let history = self.state.ohlcv_history.read().await;
        if let Some(bars) = history.get(symbol) {
            if bars.len() >= 20 {
                let profile = compute_volume_profile(bars, 20);
                let current_price = bars.last().unwrap().close;
                // Position relative to value area: inside = balanced, above/below = directional
                let relative = if current_price > profile.vah {
                    1.0 // above value area = bullish
                } else if current_price < profile.val {
                    -1.0 // below value area = bearish
                } else {
                    0.0 // inside value area = balanced
                };
                return (profile.poc, profile.vah, profile.val, relative);
            }
        }
        (0.0, 0.0, 0.0, 0.0)
    }
}

#[async_trait]
impl AgentSkill for VolumeProfileSkill {
    fn name(&self) -> &str {
        "VolumeProfile"
    }
    fn description(&self) -> &str {
        "Computes Point of Control (POC), Value Area High (VAH), and Value Area Low (VAL) from volume distribution. Price above VAH = bullish, below VAL = bearish."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (poc, vah, val, relative) = self.analyze(&context.symbol).await;
            let note = format!(
                "POC={:.2} VAH={:.2} VAL={:.2} pos={}",
                poc,
                vah,
                val,
                if relative > 0.0 {
                    "above VAH"
                } else if relative < 0.0 {
                    "below VAL"
                } else {
                    "inside VA"
                }
            );
            println!("[Skill] {} for {}: {}", self.name(), context.symbol, note);
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score: (relative + 1.0) / 2.0, // map [-1, 1] to [0, 1]
                note,
                confidence: if poc > 0.0 { 0.75 } else { 0.4 },
                direction: if relative > 0.0 {
                    tredo_core::agent::SkillDirection::Bullish
                } else if relative < 0.0 {
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
