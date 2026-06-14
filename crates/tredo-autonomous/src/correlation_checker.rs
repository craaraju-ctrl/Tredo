// CorrelationChecker (pairs skill/tool)
// For pairs trading / hedging awareness. Research: Enhances mean-reversion and risk in correlated assets (crypto focus).
// Now implements AgentSkill for pluggability. Stub improved with basic cross-symbol proxy when history available.

use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{skills::AgentSkill, AgentInput, AgentOutput};

pub struct CorrelationChecker {
    pub state: SharedState,
}

impl CorrelationChecker {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn check_correlation(&self, symbol: &str) -> f64 {
        let history = self.state.ohlcv_history.read().await;

        // If we have history for major crypto pairs, compute a crude proxy correlation vs "BTC" or average.
        // Real version would keep aligned time-series and use Pearson.
        let majors = ["BTC", "ETH", "SOL"];
        if majors.contains(&symbol) {
            // High baseline corr among major cryptos (typical >0.6-0.8 in practice)
            if let (Some(bars), Some(btc_bars)) = (history.get(symbol), history.get("BTC")) {
                if bars.len() >= 10 && btc_bars.len() >= 10 {
                    // Simple proxy: if recent price moves directionally aligned with BTC, high corr
                    let sym_change = (bars.last().unwrap().close - bars[bars.len() - 5].close)
                        / bars[bars.len() - 5].close;
                    let btc_change = (btc_bars.last().unwrap().close
                        - btc_bars[btc_bars.len() - 5].close)
                        / btc_bars[btc_bars.len() - 5].close;
                    let aligned = (sym_change * btc_change) > 0.0;
                    return if aligned { 0.78 } else { 0.55 };
                }
            }
            return 0.72; // default high for majors
        }

        // Fallback symbol-based
        match symbol {
            "BTC" | "ETH" | "SOL" => 0.75,
            _ => 0.5,
        }
    }
}

#[async_trait]
impl AgentSkill for CorrelationChecker {
    fn name(&self) -> &str {
        "CorrelationChecker"
    }
    fn description(&self) -> &str {
        "Estimates correlation to major assets (esp. BTC for crypto) using recent price history (how to detect pair risk / fakeouts for hedging or caution)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let corr = self.check_correlation(&context.symbol).await;
            println!(
                "[Skill] {} executed for {}: corr={:.2}",
                self.name(),
                context.symbol,
                corr
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score: corr,
                note: "pair correlation proxy".to_string(),
                confidence: 0.65,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
