use async_trait::async_trait;
use std::error::Error;
use tredo_autonomous::SharedState;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier};

/// Delegates to `tredo_autonomous::portfolio_manager::PortfolioManagerAgent`.
pub struct PortfolioManagerAgent {
    inner: tredo_autonomous::portfolio_manager::PortfolioManagerAgent,
}

impl PortfolioManagerAgent {
    pub fn new(state: SharedState) -> Self {
        Self {
            inner: tredo_autonomous::portfolio_manager::PortfolioManagerAgent::new(state),
        }
    }
}

#[async_trait]
impl Agent for PortfolioManagerAgent {
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
