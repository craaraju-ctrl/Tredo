// orchestrator_struct.rs
// AutonomousOrchestrator struct definition + new() + record_result()

use crate::state::SharedState;
use crate::tredo::Tredo;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AutonomousOrchestrator {
    pub state: SharedState,
    pub tredo: Option<Tredo>,
    pub market_intel: Arc<crate::market_intelligence::MarketIntelligenceAgent>,
    pub risk_psych: Arc<crate::risk_psychology::RiskPsychologyAgent>,
    pub reflector: Arc<crate::reflector::ReflectorAgent>,
    pub strategy: Arc<crate::strategy_decision::StrategyDecisionAgent>,
    pub portfolio: Arc<crate::portfolio_manager::PortfolioManagerAgent>,
    pub execution: Arc<crate::execution_coordinator::ExecutionCoordinatorAgent>,
    pub risk_calc: Arc<crate::risk_calculator::RiskCalculatorAgent>,
    pub pivot_calc: Arc<crate::pivot_calculator::PivotCalculatorAgent>,
    pub confluence: Arc<crate::confluence_scorer::ConfluenceScorerAgent>,
    pub session_timer: Arc<crate::session_timer::SessionTimerAgent>,
    pub drawdown: Arc<crate::drawdown_monitor::DrawdownMonitorAgent>,
    pub red_folder: Arc<crate::red_folder_checker::RedFolderCheckerAgent>,
    pub overtrading: Arc<crate::overtrading_preventer::OvertradingPreventerAgent>,
    pub outcome_logger: Arc<crate::outcome_logger::OutcomeLoggerAgent>,
    pub pattern_retriever: Arc<crate::pattern_retriever::PatternRetrieverAgent>,
    pub scanner: Arc<crate::scanner::WatchlistScannerAgent>,
    pub results: Arc<RwLock<Vec<crate::types::PipelineResult>>>,
}

impl AutonomousOrchestrator {
    /// Convenience accessor for the Tredo agent hierarchy.
    /// Panics if init_tredo() has not been called after construction.
    pub fn tredo(&self) -> &Tredo {
        self.tredo
            .as_ref()
            .expect("Tredo not initialized — call init_tredo() after construction")
    }

    /// Initialize the Tredo agent hierarchy (must be called once after construction).
    /// Uses Arc::clone() for zero-copy sharing — no agent state is duplicated.
    pub fn init_tredo(&mut self) {
        self.tredo = Some(Tredo::from_orchestrator(self));
    }

    pub fn new(state: SharedState) -> Self {
        Self {
            market_intel: Arc::new(crate::market_intelligence::MarketIntelligenceAgent::new(
                state.clone(),
            )),
            risk_psych: Arc::new(crate::risk_psychology::RiskPsychologyAgent::new(
                state.clone(),
            )),
            reflector: Arc::new(crate::reflector::ReflectorAgent::new(state.clone())),
            strategy: Arc::new(crate::strategy_decision::StrategyDecisionAgent::new(
                state.clone(),
            )),
            portfolio: Arc::new(crate::portfolio_manager::PortfolioManagerAgent::new(
                state.clone(),
            )),
            execution: Arc::new(
                crate::execution_coordinator::ExecutionCoordinatorAgent::new(state.clone()),
            ),
            risk_calc: Arc::new(crate::risk_calculator::RiskCalculatorAgent::new(
                state.clone(),
            )),
            pivot_calc: Arc::new(crate::pivot_calculator::PivotCalculatorAgent::new(
                state.clone(),
            )),
            confluence: Arc::new(crate::confluence_scorer::ConfluenceScorerAgent::new(
                state.clone(),
            )),
            session_timer: Arc::new(crate::session_timer::SessionTimerAgent::new(state.clone())),
            drawdown: Arc::new(crate::drawdown_monitor::DrawdownMonitorAgent::new(
                state.clone(),
            )),
            red_folder: Arc::new(crate::red_folder_checker::RedFolderCheckerAgent::new(
                state.clone(),
            )),
            overtrading: Arc::new(
                crate::overtrading_preventer::OvertradingPreventerAgent::new(state.clone()),
            ),
            outcome_logger: Arc::new(crate::outcome_logger::OutcomeLoggerAgent::new(
                state.clone(),
            )),
            pattern_retriever: Arc::new(crate::pattern_retriever::PatternRetrieverAgent::new(
                state.clone(),
            )),
            scanner: Arc::new(crate::scanner::WatchlistScannerAgent::new(state.clone())),
            results: Arc::new(RwLock::new(Vec::new())),
            state,
            tredo: None, // set to Some(Tredo::from_orchestrator(self)) after construction
        }
    }

    pub async fn record_result(
        &self,
        phase: &str,
        passed: bool,
        details: Vec<String>,
        duration_ms: u64,
    ) {
        let mut results = self.results.write().await;
        results.push(crate::types::PipelineResult {
            phase: phase.to_string(),
            passed,
            details,
            duration_ms,
        });
    }
}
