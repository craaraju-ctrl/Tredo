// backtester.rs
// Implementation of AutonomousBacktester for simulated historical runs

use crate::orchestrator_struct::AutonomousOrchestrator;
use std::error::Error;
use tredo_core::{MarketContext, TradeDirection};

pub struct AutonomousBacktester {
    pub orchestrator: AutonomousOrchestrator,
}

impl AutonomousBacktester {
    pub fn new(orchestrator: AutonomousOrchestrator) -> Self {
        Self { orchestrator }
    }

    pub async fn run_backtest(
        &self,
        symbol: &str,
        direction: TradeDirection,
        data: Vec<MarketContext>,
        slippage_pct: f64, // Research upgrade: realistic slippage (e.g. 0.001 = 0.1%)
    ) -> Result<AutonomousBacktestResult, Box<dyn Error + Send + Sync>> {
        let mut summaries = Vec::new();
        let mut total_pnl = 0.0;
        for ctx in data {
            // Apply slippage to entry for realism (research-backed backtesting)
            let slip = ctx.current_price * slippage_pct;
            let adjusted_entry = if direction == TradeDirection::Long {
                ctx.current_price + slip
            } else {
                ctx.current_price - slip
            };
            let sl = adjusted_entry * 0.99;
            let tp = adjusted_entry * 1.02;

            let summary = self
                .orchestrator
                .run_full_pipeline(symbol)  // agentic: agent decides levels itself
                .await?;
            summaries.push(summary.clone());

            if summary.executed {
                // Rough P&L with slippage (exit also slipped)
                let exit_slip = adjusted_entry * 0.001; // exit slip
                let exit_price = if direction == TradeDirection::Long {
                    adjusted_entry * 1.02 - exit_slip
                } else {
                    adjusted_entry * 0.98 + exit_slip
                };
                let pnl = if direction == TradeDirection::Long {
                    (exit_price - adjusted_entry) * 10.0 // assume 10 units
                } else {
                    (adjusted_entry - exit_price) * 10.0
                };
                total_pnl += pnl;
            }
        }

        let total_runs = summaries.len();
        let executed_count = summaries.iter().filter(|s| s.executed).count();

        Ok(AutonomousBacktestResult {
            total_runs,
            executed_count,
            message: format!("Autonomous backtest (slippage {}%) finished. Runs: {}, Executed: {}, Est PnL: {:.2}", slippage_pct*100.0, total_runs, executed_count, total_pnl),
            total_pnl, // upgraded result
        })
    }
}

#[derive(Debug, Clone)]
pub struct AutonomousBacktestResult {
    pub total_runs: usize,
    pub executed_count: usize,
    pub message: String,
    pub total_pnl: f64,
}
