use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::error::Error;

use crate::disciplined_core::{DisciplineCheck, MarketContext, PivotLevels};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTier {
    Main,
    Sub,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentInput {
    PivotRequest { high: f64, low: f64, close: f64 },
    ConfluenceRequest { context: MarketContext },
    RiskRequest { context: MarketContext },
    LogOutcome { key: String, value: String },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentOutput {
    PivotResult(PivotLevels),
    ConfluenceResult(f64),
    RiskResult(DisciplineCheck),
    SkillResult {
        name: String,
        score: f64,
        note: String,
        confidence: f64,
    },
    Done,
    NoOutput,
}

#[async_trait]
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn tier(&self) -> AgentTier;
    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>>;
}

impl AgentOutput {
    pub fn is_ok(&self) -> bool {
        match self {
            AgentOutput::RiskResult(check) => check.passed,
            AgentOutput::Done | AgentOutput::NoOutput => true,
            _ => true,
        }
    }
}
