use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRole {
    MarketIntelligence,
    RiskPsychology,
    Reflector,
    StrategyDecision,
    PortfolioManager,
    ExecutionCoordinator,
    SubAgent,
}

impl AgentRole {
    pub fn description(&self) -> &'static str {
        match self {
            AgentRole::MarketIntelligence => "Market Intelligence",
            AgentRole::RiskPsychology => "Risk Psychology",
            AgentRole::Reflector => "Reflector",
            AgentRole::StrategyDecision => "Strategy Decision",
            AgentRole::PortfolioManager => "Portfolio Manager",
            AgentRole::ExecutionCoordinator => "Execution Coordinator",
            AgentRole::SubAgent => "Sub Agent",
        }
    }
}
