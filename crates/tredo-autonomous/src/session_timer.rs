use crate::helpers::get_indian_session_info;
use crate::state::SharedState;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier, DisciplineCheck};

/// SUB AGENT 4 — SessionTimerAgent (Deterministic)
/// Monitors Indian market session (NSE/BSE) and enforces session rules
pub struct SessionTimerAgent {
    pub state: SharedState,
}

impl SessionTimerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    fn is_nse_session_active(&self) -> (bool, String) {
        let session = get_indian_session_info(Utc::now());

        if !session.market_open {
            return (
                false,
                format!(
                    "Market CLOSED | Session: {} | Next open: {:?}",
                    session.session_name, session.time_to_open
                ),
            );
        }

        // Avoid first 15 minutes (high volatility) and last 15 minutes (unwinds)
        if session.minutes_since_open < 15 {
            return (
                false,
                format!(
                    "WAIT | Only {} mins since open - avoid opening volatility",
                    session.minutes_since_open
                ),
            );
        }

        if let Some(mins) = session.time_to_close {
            if mins < 15 {
                return (
                    false,
                    format!("WAIT | Only {} mins to close - avoid closing unwinds", mins),
                );
            }
            return (
                true,
                format!(
                    "ACTIVE | {} | Mins since open: {} | Mins to close: {}",
                    session.session_name, session.minutes_since_open, mins
                ),
            );
        }

        (true, format!("ACTIVE | {}", session.session_name))
    }
}

#[async_trait]
impl Agent for SessionTimerAgent {
    fn name(&self) -> &str {
        "SessionTimerAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let (active, status) = self.is_nse_session_active();

        println!("[SessionTimer] {}", status);

        let check = DisciplineCheck {
            passed: active,
            reasons: if active { vec![] } else { vec![status.clone()] },
            confluence_score: None,
        };

        Ok(AgentOutput::RiskResult(check))
    }
}
