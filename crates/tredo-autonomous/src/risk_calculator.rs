use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{check_risk_limits, Agent, AgentInput, AgentOutput, AgentTier};

pub struct RiskCalculatorAgent {
    pub state: SharedState,
}

impl RiskCalculatorAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Agent for RiskCalculatorAgent {
    fn name(&self) -> &str {
        "RiskCalculatorAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let Some(AgentInput::RiskRequest { context }) = input {
            let rules = self.state.rules.read().await;
            let check = check_risk_limits(&context, &rules);
            println!("[RiskCalculator] Risk check completed");
            Ok(AgentOutput::RiskResult(check))
        } else {
            Ok(AgentOutput::NoOutput)
        }
    }
}
