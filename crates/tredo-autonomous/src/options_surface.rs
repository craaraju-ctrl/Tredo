// OptionsSurfaceSkill — options chain analysis (PCR, skew, max pain, Greeks signals).
// Implements AgentSkill. Uses OPTIONS_CHAIN_{SYMBOL} env JSON or built-in stub for index symbols.

use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{analyze_options_chain, AgentInput, AgentOutput, OptionsChain};

pub struct OptionsSurfaceSkill {
    #[allow(dead_code)]
    pub state: SharedState,
}

impl OptionsSurfaceSkill {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    fn load_chain(symbol: &str, spot: f64) -> Option<OptionsChain> {
        let env_key = format!("OPTIONS_CHAIN_{}", symbol.to_uppercase());
        if let Ok(json) = std::env::var(&env_key).or_else(|_| std::env::var("OPTIONS_CHAIN_JSON")) {
            if let Ok(mut chain) = serde_json::from_str::<OptionsChain>(&json) {
                if chain.symbol.is_empty() {
                    chain.symbol = symbol.to_string();
                }
                if chain.underlying_price <= 0.0 {
                    chain.underlying_price = spot;
                }
                return Some(chain);
            }
        }

        // Index symbols without live chain data — neutral (no fabricated signals)
        match symbol.to_uppercase().as_str() {
            "NIFTY" | "BANKNIFTY" | "FINNIFTY" | "SENSEX" => None,
            _ => None,
        }
    }

    pub fn analyze_sync(
        symbol: &str,
        spot: f64,
    ) -> (f64, String, tredo_core::agent::SkillDirection) {
        let Some(chain) = Self::load_chain(symbol, spot) else {
            return (
                0.5,
                "no options chain configured".into(),
                tredo_core::agent::SkillDirection::Neutral,
            );
        };

        let signals = analyze_options_chain(&chain);
        if signals.is_empty() {
            return (
                0.5,
                "options chain loaded — no actionable signals".into(),
                tredo_core::agent::SkillDirection::Neutral,
            );
        }

        let bullish: f64 = signals
            .iter()
            .filter(|s| s.direction == "bullish")
            .map(|s| s.confidence)
            .sum();
        let bearish: f64 = signals
            .iter()
            .filter(|s| s.direction == "bearish")
            .map(|s| s.confidence)
            .sum();
        let net = bullish - bearish;
        let score = (0.5 + net * 0.25).clamp(0.0, 1.0);
        let direction = if net > 0.15 {
            tredo_core::agent::SkillDirection::Bullish
        } else if net < -0.15 {
            tredo_core::agent::SkillDirection::Bearish
        } else {
            tredo_core::agent::SkillDirection::Neutral
        };
        let note = signals
            .iter()
            .take(2)
            .map(|s| format!("{}:{}", s.signal_type, s.direction))
            .collect::<Vec<_>>()
            .join(", ");
        (score, note, direction)
    }
}

#[async_trait]
impl AgentSkill for OptionsSurfaceSkill {
    fn name(&self) -> &str {
        "OptionsSurface"
    }
    fn description(&self) -> &str {
        "Analyzes options chain surface (PCR, skew, max pain). Set OPTIONS_CHAIN_{SYMBOL} JSON for live data."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let (score, note, direction) =
                Self::analyze_sync(&context.symbol, context.current_price);
            let confidence = if note.contains("no options") {
                0.3
            } else {
                0.65
            };
            println!(
                "[Skill] {} for {}: score={:.2} {}",
                self.name(),
                context.symbol,
                score,
                note
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note,
                confidence,
                direction,
                weight: 0.0,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
