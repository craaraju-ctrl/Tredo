use crate::state::SharedState;
use crate::types::TradeSignal;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier};

pub struct PortfolioManagerAgent {
    pub state: SharedState,
}

impl PortfolioManagerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    async fn assess_portfolio(&self) -> Result<String, Box<dyn Error + Send + Sync>> {
        let portfolio = self.state.portfolio.read().await;

        let total_exposure: f64 = portfolio
            .open_positions
            .iter()
            .map(|p| p.quantity * p.current_price)
            .sum();

        let exposure_pct = if portfolio.total_equity > 0.0 {
            total_exposure / portfolio.total_equity * 100.0
        } else {
            0.0
        };

        let assessment = format!(
            "Portfolio: Equity ₹{:.2} | Cash ₹{:.2} | Exposure {:.1}% | Positions {} | Today P&L: ₹{:.2}",
            portfolio.total_equity, portfolio.cash_balance, exposure_pct,
            portfolio.open_positions.len(), portfolio.daily_pnl
        );

        println!("[PortfolioManager] {}", assessment);
        Ok(assessment)
    }

    pub async fn update_position_pnl(
        &self,
        symbol: &str,
        current_price: f64,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let mut portfolio = self.state.portfolio.write().await;

        let mut pnl = 0.0;
        let mut updated = false;
        for pos in &mut portfolio.open_positions {
            if pos.symbol == symbol {
                pos.current_price = current_price;
                pos.unrealized_pnl = match pos.direction {
                    tredo_core::TradeDirection::Long => {
                        (current_price - pos.entry_price) * pos.quantity
                    }
                    tredo_core::TradeDirection::Short => {
                        (pos.entry_price - current_price) * pos.quantity
                    }
                };
                pos.unrealized_pnl_pct = if pos.entry_price > 0.0 {
                    pos.unrealized_pnl / (pos.entry_price * pos.quantity) * 100.0
                } else {
                    0.0
                };
                pnl = pos.unrealized_pnl;
                updated = true;
                break;
            }
        }

        if updated {
            let open_value: f64 = portfolio
                .open_positions
                .iter()
                .map(|p| match p.direction {
                    tredo_core::TradeDirection::Long => p.quantity * p.current_price,
                    tredo_core::TradeDirection::Short => {
                        (p.quantity * p.entry_price) + p.unrealized_pnl
                    }
                })
                .sum();
            portfolio.total_equity = portfolio.cash_balance + open_value;
        }

        Ok(pnl)
    }

    pub async fn add_position(
        &self,
        signal: &TradeSignal,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut portfolio = self.state.portfolio.write().await;

        if !portfolio.trading_enabled {
            return Err("Trading is disabled/halted".into());
        }

        if portfolio
            .open_positions
            .iter()
            .any(|p| p.symbol == signal.symbol)
        {
            return Err("Position already open for this symbol".into());
        }

        let position_value = signal.position_size * signal.entry_price;
        if position_value > portfolio.cash_balance * 0.95 {
            return Err("Insufficient cash for position".into());
        }

        let risk_amount = signal.position_size * (signal.entry_price - signal.stop_loss).abs();

        let position = crate::types::OpenPosition {
            symbol: signal.symbol.clone(),
            direction: signal.direction,
            entry_price: signal.entry_price,
            current_price: signal.entry_price,
            stop_loss: signal.stop_loss,
            take_profit: signal.take_profit,
            quantity: signal.position_size,
            unrealized_pnl: 0.0,
            unrealized_pnl_pct: 0.0,
            entry_time: Utc::now(),
            risk_amount,
        };

        portfolio.cash_balance -= position_value;
        portfolio.open_positions.push(position);
        portfolio.total_trades_today += 1;
        portfolio.last_trade_time = Some(Utc::now());

        let open_value: f64 = portfolio
            .open_positions
            .iter()
            .map(|p| match p.direction {
                tredo_core::TradeDirection::Long => p.quantity * p.current_price,
                tredo_core::TradeDirection::Short => {
                    (p.quantity * p.entry_price) + p.unrealized_pnl
                }
            })
            .sum();
        portfolio.total_equity = portfolio.cash_balance + open_value;

        println!(
            "[PortfolioManager] Added position: {} {} {:.0} shares @ {:.2}",
            signal.symbol,
            if signal.direction == tredo_core::TradeDirection::Long {
                "LONG"
            } else {
                "SHORT"
            },
            signal.position_size,
            signal.entry_price
        );

        Ok(())
    }

    pub async fn close_position(
        &self,
        symbol: &str,
        exit_price: f64,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        let mut portfolio = self.state.portfolio.write().await;

        if let Some(idx) = portfolio
            .open_positions
            .iter()
            .position(|p| p.symbol == symbol)
        {
            let pos = portfolio.open_positions.remove(idx);

            let realized_pnl = match pos.direction {
                tredo_core::TradeDirection::Long => (exit_price - pos.entry_price) * pos.quantity,
                tredo_core::TradeDirection::Short => (pos.entry_price - exit_price) * pos.quantity,
            };

            portfolio.cash_balance += (pos.quantity * pos.entry_price) + realized_pnl;
            portfolio.daily_pnl += realized_pnl;

            if realized_pnl > 0.0 {
                portfolio.winning_trades_today += 1;
                portfolio.consecutive_losses = 0;
            } else {
                portfolio.losing_trades_today += 1;
                portfolio.consecutive_losses += 1;
            }

            let open_value: f64 = portfolio
                .open_positions
                .iter()
                .map(|p| match p.direction {
                    tredo_core::TradeDirection::Long => p.quantity * p.current_price,
                    tredo_core::TradeDirection::Short => {
                        (p.quantity * p.entry_price) + p.unrealized_pnl
                    }
                })
                .sum();
            portfolio.total_equity = portfolio.cash_balance + open_value;

            if portfolio.daily_pnl < 0.0 {
                let dd = portfolio.daily_pnl.abs() / portfolio.total_equity;
                if dd > portfolio.max_drawdown_today {
                    portfolio.max_drawdown_today = dd;
                }
            }

            let rules = self.state.rules.read().await;
            if portfolio.max_drawdown_today >= rules.max_daily_drawdown
                || portfolio.consecutive_losses >= rules.max_consecutive_losses
            {
                portfolio.trading_enabled = false;
                println!("[PortfolioManager] TRADING HALTED");
            }

            println!(
                "[PortfolioManager] Closed {} @ {:.2} | P&L: ₹{:.2}",
                symbol, exit_price, realized_pnl
            );
            return Ok(realized_pnl);
        }

        Err("Position not found".into())
    }
}

#[async_trait]
impl Agent for PortfolioManagerAgent {
    fn name(&self) -> &str {
        "PortfolioManagerAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let _ = self.assess_portfolio().await;
        Ok(AgentOutput::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TradeSignal;
    use chrono::Utc;
    use std::fs;
    use tredo_core::{Config, DisciplineRules, MemoryStore, TradeDirection};

    #[tokio::test]
    async fn test_short_position_accounting() {
        let db_path = "test_portfolio_memory.redb";
        let _ = fs::remove_file(db_path);

        let memory = MemoryStore::new(db_path)
            .expect("Portfolio operation failed - check for concurrent access or DB issues");
        let config = Config::default();
        let rules = DisciplineRules::default();
        let state = SharedState::new(memory, rules, config, "test_portfolio_history.db")
            .expect("SharedState init (episode DB)");

        let pm = PortfolioManagerAgent::new(state.clone());

        // Initial state
        {
            let portfolio = state.portfolio.read().await;
            assert_eq!(portfolio.cash_balance, 100_000.0);
            assert_eq!(portfolio.total_equity, 100_000.0);
        }

        // Add short position of NIFTY
        let signal = TradeSignal {
            symbol: "NIFTY".to_string(),
            direction: TradeDirection::Short,
            entry_price: 100.0,
            stop_loss: 110.0,
            take_profit: 80.0,
            position_size: 10.0,
            confidence_score: 0.8,
            confluence_score: 0.8,
            risk_reward_ratio: 2.0,
            reasoning: "Test short".to_string(),
            timestamp: Utc::now(),
            session_valid: true,
            risk_check_passed: true,
        };

        pm.add_position(&signal)
            .await
            .expect("Portfolio operation failed - check for concurrent access or DB issues");

        // After entry
        {
            let portfolio = state.portfolio.read().await;
            assert_eq!(portfolio.cash_balance, 99_000.0);
            assert_eq!(portfolio.total_equity, 100_000.0);
            assert_eq!(portfolio.open_positions.len(), 1);
        }

        // Move price in favor of short (to 80)
        pm.update_position_pnl("NIFTY", 80.0)
            .await
            .expect("Portfolio operation failed - check for concurrent access or DB issues");

        // Equity should increase as short is in profit
        {
            let portfolio = state.portfolio.read().await;
            assert_eq!(portfolio.total_equity, 100_200.0);
        }

        // Close short position at 80
        let pnl = pm
            .close_position("NIFTY", 80.0)
            .await
            .expect("Portfolio operation failed - check for concurrent access or DB issues");
        assert_eq!(pnl, 200.0);

        // After close, cash and equity should be updated correctly
        {
            let portfolio = state.portfolio.read().await;
            assert_eq!(portfolio.cash_balance, 100_200.0);
            assert_eq!(portfolio.total_equity, 100_200.0);
            assert!(portfolio.open_positions.is_empty());
        }

        let _ = fs::remove_file(db_path);
    }
}
