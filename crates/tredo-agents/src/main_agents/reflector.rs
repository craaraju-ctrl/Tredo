use async_trait::async_trait;
use std::error::Error;
use tredo_autonomous::SharedState;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier};

/// Delegates to `tredo_autonomous::reflector::ReflectorAgent`.
pub struct ReflectorAgent {
    inner: tredo_autonomous::reflector::ReflectorAgent,
}

impl ReflectorAgent {
    pub fn new(state: SharedState) -> Self {
        Self {
            inner: tredo_autonomous::reflector::ReflectorAgent::new(state),
        }
    }
}

#[async_trait]
impl Agent for ReflectorAgent {
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
