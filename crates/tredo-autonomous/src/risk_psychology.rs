use crate::state::SharedState;
use crate::types::{RiskAnalysis, RiskRecommendation};
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{check_risk_limits, Agent, AgentInput, AgentOutput, AgentTier, MarketContext};

pub struct RiskPsychologyAgent {
    pub state: SharedState,
}

impl RiskPsychologyAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn analyze_risk(
        &self,
        _context: &MarketContext,
    ) -> Result<RiskAnalysis, Box<dyn Error + Send + Sync>> {
        let portfolio = self.state.portfolio.read().await;
        let rules = self.state.rules.read().await;

        let total_risk: f64 = portfolio.open_positions.iter().map(|p| p.risk_amount).sum();
        let portfolio_heat = if portfolio.total_equity > 0.0 {
            total_risk / portfolio.total_equity
        } else {
            0.0
        };

        let daily_dd = if portfolio.daily_pnl < 0.0 && portfolio.total_equity > 0.0 {
            portfolio.daily_pnl.abs() / portfolio.total_equity
        } else {
            0.0
        };

        let mut psych_warnings = Vec::new();

        if portfolio.consecutive_losses >= 2 {
            psych_warnings.push(format!(
                "\u{26a0}\u{fe0f} {} consecutive losses - risk of revenge trading",
                portfolio.consecutive_losses
            ));
        }

        if portfolio.total_trades_today >= 5 {
            psych_warnings.push(format!(
                "\u{26a0}\u{fe0f} {} trades today - approaching overtrading threshold",
                portfolio.total_trades_today
            ));
        }

        if daily_dd >= rules.max_daily_drawdown * 0.7 {
            psych_warnings.push(format!(
                "\u{26a0}\u{fe0f} Daily drawdown {:.1}% approaching limit {:.1}%",
                daily_dd * 100.0,
                rules.max_daily_drawdown * 100.0
            ));
        }

        let recommendation = if !portfolio.trading_enabled
            || daily_dd >= rules.max_daily_drawdown
            || portfolio.consecutive_losses >= rules.max_consecutive_losses
        {
            RiskRecommendation::Halt
        } else if daily_dd >= rules.max_daily_drawdown * 0.7 || portfolio_heat > 0.15 {
            RiskRecommendation::ReduceSize
        } else {
            RiskRecommendation::Proceed
        };

        println!(
            "[RiskPsychology] {:?} | Heat: {:.1}% | DD: {:.1}% | Trades today: {}",
            recommendation,
            portfolio_heat * 100.0,
            daily_dd * 100.0,
            portfolio.total_trades_today
        );

        for warning in &psych_warnings {
            println!("[RiskPsychology] {}", warning);
        }

        Ok(RiskAnalysis {
            max_position_size: rules.max_risk_per_trade * portfolio.total_equity,
            risk_per_trade_pct: rules.max_risk_per_trade,
            risk_reward_ratio: 0.0,
            portfolio_heat,
            daily_drawdown_pct: daily_dd,
            var_95: daily_dd * 1.65,
            recommendation,
            psychology_warnings: psych_warnings,
        })
    }
}

#[async_trait]
impl Agent for RiskPsychologyAgent {
    fn name(&self) -> &str {
        "RiskPsychologyAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        // Read real portfolio equity for accurate drawdown limit checks
        let portfolio_equity = {
            let portfolio = self.state.portfolio.read().await;
            portfolio.total_equity
        };
        let ctx = match input {
            Some(AgentInput::RiskRequest { context }) => context,
            _ => MarketContext {
                symbol: "NIFTY".to_string(),
                current_price: 24500.0,
                high: 24550.0,
                low: 24450.0,
                previous_close: 24480.0,
                timestamp: Utc::now(),
                daily_pnl: 0.0,
                equity: portfolio_equity,
                consecutive_losses: 0,
                is_red_folder_day: false,
                trend_direction: None,
            },
        };

        let analysis = self.analyze_risk(&ctx).await?;
        let mut final_check = check_risk_limits(&ctx, &*self.state.rules.read().await);

        if analysis.recommendation == RiskRecommendation::Halt {
            final_check.passed = false;
            final_check
                .reasons
                .push("RiskPsychology: Halt recommendation".to_string());
        }

        Ok(AgentOutput::RiskResult(final_check))
    }
}
