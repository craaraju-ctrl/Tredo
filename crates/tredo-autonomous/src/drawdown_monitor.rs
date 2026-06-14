use async_trait::async_trait;
use std::error::Error;

use crate::state::SharedState;
use tredo_core::{Agent, AgentInput, AgentOutput, DisciplineCheck};

/// SUB AGENT 5 — DrawdownMonitorAgent (Deterministic)
/// Monitors daily drawdown and triggers halt when limits are breached
pub struct DrawdownMonitorAgent {
    pub state: SharedState,
}

impl DrawdownMonitorAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    async fn check_drawdown(&self) -> (f64, f64, bool) {
        let portfolio = self.state.portfolio.read().await;
        let rules = self.state.rules.read().await;

        let current_dd = portfolio.max_drawdown_today;
        let limit = rules.max_daily_drawdown;
        let halted = !portfolio.trading_enabled;

        (current_dd, limit, halted)
    }
}

#[async_trait]
impl Agent for DrawdownMonitorAgent {
    fn name(&self) -> &str {
        "DrawdownMonitorAgent"
    }
    fn tier(&self) -> tredo_core::AgentTier {
        tredo_core::AgentTier::Sub
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let (current_dd, limit, halted) = self.check_drawdown().await;

        let pct = current_dd * 100.0;
        let limit_pct = limit * 100.0;
        let remaining = limit_pct - pct;

        if halted {
            println!(
                "[DrawdownMonitor] 👋 TRADING HALTED | Drawdown: {:.2}% | Limit: {:.2}%",
                pct, limit_pct
            );
        } else if pct >= limit_pct * 0.8 {
            println!("[DrawdownMonitor] ⚠️ WARNING | Drawdown: {:.2}% | Remaining: {:.2}% | Limit: {:.2}%", 
                pct, remaining, limit_pct);
        } else {
            println!(
                "[DrawdownMonitor] ✅ OK | Drawdown: {:.2}% | Remaining: {:.2}% | Limit: {:.2}%",
                pct, remaining, limit_pct
            );
        }

        let check = DisciplineCheck {
            passed: !halted,
            reasons: if halted {
                vec![format!(
                    "Drawdown limit reached: {:.2}% / {:.2}%",
                    pct, limit_pct
                )]
            } else {
                vec![]
            },
            confluence_score: None,
        };

        Ok(AgentOutput::RiskResult(check))
    }
}
