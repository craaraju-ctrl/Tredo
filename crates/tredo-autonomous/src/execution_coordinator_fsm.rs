use std::time::{SystemTime, UNIX_EPOCH};
use crate::state::SharedState;
use crate::regime_classifier::RegimeClassifier;
use crate::debate_orchestrator::DebateOrchestrator;
use crate::skills::{SkillResult, ConfluenceScorer};
use crate::risk_guardian::{RiskGuardian, ProposedTrade, GuardianPortfolioContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestratorState {
    Idle,
    Scanning,
    Evaluating,
    InPosition,
    EmergencyStop,
}

pub struct ExecutionCoordinator {
    pub state: OrchestratorState,
    pub symbol: String,
    pub regime_classifier: RegimeClassifier,
    pub debate_orchestrator: DebateOrchestrator,
}

impl ExecutionCoordinator {
    pub fn new(symbol: &str, debate: DebateOrchestrator, state: SharedState) -> Self {
        Self {
            state: OrchestratorState::Idle,
            symbol: symbol.to_string(),
            regime_classifier: RegimeClassifier::new(state),
            debate_orchestrator: debate,
        }
    }

    /// Coordinates transitions through the state machine.
    /// Rejects transitions that bypass safety gates.
    pub async fn transition_and_execute(
        &mut self,
        shared: &SharedState,
        prices: &[f64],
        current_pnl: f64,
    ) -> Result<(), String> {
        let _now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Rule: Guard loop from connection dropouts first
        if prices.is_empty() {
            self.state = OrchestratorState::EmergencyStop;
            return Err(
                "EMERGENCY STOP: Empty pricing telemetry. Lost exchange feed connection."
                    .to_string(),
            );
        }

        // FSM Transition 1: Idle -> Scanning
        if self.state == OrchestratorState::Idle {
            println!("[FSM] Transitioning: Idle -> Scanning");
            self.state = OrchestratorState::Scanning;
        }

        // FSM Transition 2: Scanning -> Evaluating (with regime analysis)
        if self.state == OrchestratorState::Scanning {
            let last_price = prices[prices.len() - 1];
            let regime = self.regime_classifier.detect_regime(&self.symbol, last_price).await;
            println!("[FSM] Transitioning: Scanning -> Evaluating (Regime: {:?})", regime);
            self.state = OrchestratorState::Evaluating;
        }

        // FSM Transition 3: Evaluating -> InPosition (if signals approve)
        if self.state == OrchestratorState::Evaluating {
            let mut skills = Vec::new();
            skills.push(SkillResult {
                score: 0.65,
                confidence: 0.85,
            });

            let skill_weights = shared.get_skill_weights();
            let aggregated = ConfluenceScorer::aggregate(skills, &skill_weights);

            // Execute local debate
            let mock_memories = Vec::new();
            let outcome = self
                .debate_orchestrator
                .run_agent_debate(
                    &self.symbol,
                    prices[prices.len() - 1],
                    &aggregated,
                    &mock_memories,
                )
                .await;

            if outcome.signal_approved {
                println!("[FSM] Signal approved. Passing to Compiled Risk Guardian Firewall.");

                let proposed = ProposedTrade {
                    symbol: self.symbol.clone(),
                    entry_price: outcome.entry_level,
                    stop_loss_price: outcome.adjusted_stop_loss,
                    position_size: 0.10,
                    leverage: 3,
                };

                let safety_ctx = GuardianPortfolioContext {
                    current_drawdown_pct: current_pnl,
                    total_equity: 10000.0,
                };

                let risk_config = shared.get_risk_config();
                let guardian = RiskGuardian::new(risk_config);

                match guardian.intercept_and_validate(&proposed, &safety_ctx) {
                    Ok(_) => {
                        println!("[FSM] FIREWALL PASSED. Transitioning: Evaluating -> InPosition");
                        self.state = OrchestratorState::InPosition;
                    }
                    Err(violation) => {
                        self.state = OrchestratorState::Idle;
                        return Err(format!("FIREWALL REJECTED: {}", violation));
                    }
                }
            } else {
                println!("[FSM] Consensus HOLD. Returning state to Idle.");
                self.state = OrchestratorState::Idle;
            }
        }

        Ok(())
    }
}
