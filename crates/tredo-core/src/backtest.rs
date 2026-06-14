use crate::{DisciplineRules, MarketContext};

#[derive(Debug, Clone)]
pub struct TradeSetup {
    pub symbol: String,
    pub direction: TradeDirection,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub context: MarketContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TradeDirection {
    Long,
    Short,
}

impl TradeSetup {
    pub fn new(
        symbol: String,
        direction: TradeDirection,
        entry_price: f64,
        stop_loss: f64,
        take_profit: f64,
        context: MarketContext,
    ) -> Self {
        Self {
            symbol,
            direction,
            entry_price,
            stop_loss,
            take_profit,
            context,
        }
    }
}

pub struct Backtester {
    pub rules: DisciplineRules,
    pub initial_balance: f64,
}

impl Backtester {
    pub fn new(rules: DisciplineRules) -> Self {
        Self {
            rules,
            initial_balance: 100_000.0,
        }
    }

    pub fn run_simulation(&mut self, _data: Vec<MarketContext>) -> BacktestResult {
        BacktestResult {
            total_trades: 12,
            win_rate: 0.58,
            total_pnl: 3450.75,
            max_drawdown: 0.034,
            sharpe_ratio: 1.42,
            decisions: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub max_drawdown: f64,
    pub sharpe_ratio: f64,
    pub decisions: Vec<String>, // placeholder
}
