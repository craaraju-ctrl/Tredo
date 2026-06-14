use async_trait::async_trait;
use std::error::Error;
use tredo_autonomous::SharedState;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier};

/// Delegates to `tredo_autonomous::confluence_scorer::ConfluenceScorerAgent`.
pub struct ConfluenceScorerAgent {
    inner: tredo_autonomous::confluence_scorer::ConfluenceScorerAgent,
}

impl ConfluenceScorerAgent {
    pub fn new(state: SharedState) -> Self {
        Self {
            inner: tredo_autonomous::confluence_scorer::ConfluenceScorerAgent::new(state),
        }
    }
}

#[async_trait]
impl Agent for ConfluenceScorerAgent {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn tier(&self) -> AgentTier {
        self.inner.tier()
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        self.inner.run(input).await
    }
}
