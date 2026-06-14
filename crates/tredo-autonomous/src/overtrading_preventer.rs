use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier, DisciplineCheck};

pub struct OvertradingPreventerAgent {
    pub state: SharedState,
}

impl OvertradingPreventerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Agent for OvertradingPreventerAgent {
    fn name(&self) -> &str {
        "OvertradingPreventerAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let portfolio = self.state.portfolio.read().await;
        let rules = self.state.rules.read().await;

        let total_trades = portfolio.total_trades_today;
        let limit = rules.max_consecutive_losses * 2; // Simple heuristic limit for total trades
        let passed = total_trades < limit;

        println!(
            "[OvertradingPreventer] Trades today: {}/{} | Status: {}",
            total_trades,
            limit,
            if passed { "OK" } else { "BLOCKED" }
        );

        let check = DisciplineCheck {
            passed,
            reasons: if passed {
                vec![]
            } else {
                vec![format!(
                    "Overtrading limit reached: {} trades",
                    total_trades
                )]
            },
            confluence_score: None,
        };

        Ok(AgentOutput::RiskResult(check))
    }
}
