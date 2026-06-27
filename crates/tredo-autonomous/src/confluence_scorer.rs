use crate::state::SharedState;
use async_trait::async_trait;
use std::error::Error;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, Agent, AgentInput, AgentOutput, AgentTier,
};

pub struct ConfluenceScorerAgent {
    pub state: SharedState,
}

impl ConfluenceScorerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Agent for ConfluenceScorerAgent {
    fn name(&self) -> &str {
        "ConfluenceScorerAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        // Smarter sub-agent: even deterministic ones now use hierarchical trained memory to "understand what it (the confluence scorer) did in past similar market conditions" and adjust or log the memory-informed confluence (e.g., if past high regret on this confluence level, be more conservative in score).
        // This makes sub-agents smarter without adding complexity to their core logic.
        if let Some(AgentInput::ConfluenceRequest { context }) = &input {
            let trained = self
                .state
                .recall_trained_memory(
                    &format!(
                        "confluence score for {} at confluence level",
                        context.symbol
                    ),
                    1,
                )
                .await;
            // In real, could adjust the score here based on trained (e.g., lower if past bad). For now, the recall is available for the agent to "know" its history.
            println!("[ConfluenceScorer smarter] {}\n(Trained memory for self-understanding in sub-agent.)", trained);
        }
        if let Some(AgentInput::ConfluenceRequest { context }) = input {
            let rules = self.state.rules.read().await;
            let pivots = calculate_pivot_points(
                context.high,
                context.low,
                context.previous_close,
                rules.pivot_method,
            );
            let score = calculate_confluence_score(&context, &pivots);
            Ok(AgentOutput::ConfluenceResult(score))
        } else {
            Ok(AgentOutput::NoOutput)
        }
    }
}
