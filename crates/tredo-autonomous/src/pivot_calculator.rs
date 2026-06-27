use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{calculate_pivot_points, Agent, AgentInput, AgentOutput, AgentTier};

pub struct PivotCalculatorAgent {
    pub state: SharedState,
}

impl PivotCalculatorAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Agent for PivotCalculatorAgent {
    fn name(&self) -> &str {
        "PivotCalculatorAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let Some(AgentInput::PivotRequest { high, low, close }) = input {
            let rules = self.state.rules.read().await;
            let pivots = calculate_pivot_points(high, low, close, rules.pivot_method);
            Ok(AgentOutput::PivotResult(pivots))
        } else {
            Ok(AgentOutput::NoOutput)
        }
    }
}
