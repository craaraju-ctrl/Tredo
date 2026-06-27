use async_trait::async_trait;
use std::error::Error;

use crate::state::SharedState;
use tredo_core::{Agent, AgentInput, AgentOutput, DisciplineCheck};

/// Hard daily drawdown limit — research shows 2% is optimal for precision.
/// Beyond this, the system is statistically likely to be in a regime it doesn't understand.
const HARD_DAILY_DD_LIMIT: f64 = 0.02;

/// Cascade warning thresholds (fraction of hard limit)
const WARNING_80_PCT: f64 = 0.80; // At 1.6% DD — "reduce position sizes"
const WARNING_50_PCT: f64 = 0.50; // At 1.0% DD — "caution signal"

/// SUB AGENT 5 — DrawdownMonitorAgent (Deterministic)
/// Monitors daily drawdown with cascade warnings and hard halt.
///
/// Precision improvement: Tighter drawdown limits force the system to
/// stop trading when it's in unfamiliar territory, preventing the
/// "revenge trading" death spiral that destroys precision.
pub struct DrawdownMonitorAgent {
    pub state: SharedState,
}

impl DrawdownMonitorAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    async fn check_drawdown(&self) -> (f64, f64, bool) {
        let portfolio = self.state.portfolio.read().await;
        let current_dd = portfolio.max_drawdown_today;
        let halted = !portfolio.trading_enabled;

        // Use the tighter hard limit instead of the configurable rules limit
        (current_dd, HARD_DAILY_DD_LIMIT, halted)
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
        let remaining = (limit - current_dd).max(0.0) * 100.0;
        let ratio = current_dd / limit;

        // Cascade warning system
        let level = if halted {
            "HALTED"
        } else if ratio >= WARNING_80_PCT {
            "CRITICAL"
        } else if ratio >= WARNING_50_PCT {
            "WARNING"
        } else {
            "OK"
        };

        let emoji = match level {
            "HALTED" => "🔴",
            "CRITICAL" => "🟠",
            "WARNING" => "🟡",
            _ => "🟢",
        };

        println!(
            "[DrawdownMonitor] {} {} | DD: {:.2}% | Remaining: {:.2}% | Limit: {:.2}%",
            emoji, level, pct, remaining, limit_pct
        );

        // At CRITICAL level, actually halt trading to enforce the precision limit
        if ratio >= WARNING_80_PCT && !halted {
            let mut portfolio = self.state.portfolio.write().await;
            portfolio.trading_enabled = false;
            println!(
                "[DrawdownMonitor] 🛑 TRADING HALTED at {:.2}% — {}% of hard limit consumed.",
                pct,
                (ratio * 100.0) as u32
            );
        } else if ratio >= WARNING_50_PCT && !halted {
            println!(
                "[DrawdownMonitor] ⚠️ Position sizing will be reduced. {}% of daily limit consumed.",
                (ratio * 100.0) as u32
            );
        }

        let check = DisciplineCheck {
            passed: !halted && ratio < WARNING_80_PCT,
            reasons: if halted {
                vec![format!(
                    "Drawdown HARD HALT: {:.2}% / {:.2}%",
                    pct, limit_pct
                )]
            } else if ratio >= WARNING_80_PCT {
                vec![format!(
                    "Drawdown CRITICAL: {:.2}% / {:.2}% — reducing position sizes",
                    pct, limit_pct
                )]
            } else if ratio >= WARNING_50_PCT {
                vec![format!(
                    "Drawdown WARNING: {:.2}% / {:.2}% — caution advised",
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
