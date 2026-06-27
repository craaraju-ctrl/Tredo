use crate::{validate_trade_setup, DisciplineRules, MarketContext};

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

    /// Run a simple OHLCV-based backtest using SMA crossover logic.
    /// Iterates bar by bar through data, tracks equity and drawdown.
    pub fn run_simulation(&mut self, data: Vec<MarketContext>) -> BacktestResult {
        if data.is_empty() {
            return BacktestResult {
                total_trades: 0,
                win_rate: 0.0,
                total_pnl: 0.0,
                max_drawdown: 0.0,
                sharpe_ratio: 0.0,
                decisions: vec!["No data provided".to_string()],
            };
        }

        let mut balance = self.initial_balance;
        let mut peak_balance = self.initial_balance;
        let mut max_dd = 0.0_f64;
        let mut trades = 0u32;
        let mut wins = 0u32;
        let mut total_pnl = 0.0_f64;
        let mut returns = Vec::new();
        let mut decisions = Vec::new();

        let mut in_position = false;
        let mut entry_price = 0.0;
        let mut entry_balance = 0.0;
        let mut bars_in_trade = 0u32;

        for (i, ctx) in data.iter().enumerate() {
            if i < 20 {
                continue; // Need history for SMA
            }

            let recent: Vec<f64> = data[i.saturating_sub(20)..=i]
                .iter()
                .map(|c| c.current_price)
                .collect();
            let sma: f64 = recent.iter().sum::<f64>() / recent.len() as f64;

            // ENTRY: price crosses above SMA
            if !in_position && ctx.current_price > sma * 1.001 {
                let check = validate_trade_setup(ctx, &self.rules);
                if check.passed {
                    let risk_amount = balance * self.rules.max_risk_per_trade;
                    let sl_distance = ctx.current_price * 0.02; // 2% SL
                    let position_size = if sl_distance > 0.0 {
                        risk_amount / sl_distance
                    } else {
                        0.0
                    };
                    let position_value = position_size * ctx.current_price;

                    if position_value <= balance * 0.95 && position_size > 0.0 {
                        balance -= position_value;
                        entry_price = ctx.current_price;
                        in_position = true;
                        bars_in_trade = 0;
                        entry_balance = balance;
                        decisions.push(format!(
                            "Bar {}: BUY @ {:.2} (sma={:.2})",
                            i, ctx.current_price, sma
                        ));
                    }
                }
            }
            // EXIT: SL, TP, or 10 bars
            else if in_position {
                bars_in_trade += 1;
                let pnl_change = ctx.current_price - entry_price;
                let trade_pnl = pnl_change * (entry_balance / entry_price);

                let should_exit = ctx.current_price <= entry_price * 0.98  // SL
                    || ctx.current_price >= entry_price * 1.04  // TP
                    || bars_in_trade >= 10; // Time-based

                if should_exit {
                    balance += entry_balance + trade_pnl;
                    total_pnl += trade_pnl;
                    trades += 1;
                    if trade_pnl > 0.0 {
                        wins += 1;
                    }
                    returns.push(trade_pnl / entry_price);

                    if balance > peak_balance {
                        peak_balance = balance;
                    }
                    let dd = (peak_balance - balance) / peak_balance;
                    if dd > max_dd {
                        max_dd = dd;
                    }

                    in_position = false;

                    decisions.push(format!(
                        "Bar {}: EXIT @ {:.2} | P&L: {:+.2} | {}",
                        i,
                        ctx.current_price,
                        trade_pnl,
                        if trade_pnl > 0.0 { "WIN" } else { "LOSS" }
                    ));
                }
            }

            // Track peak balance for drawdown
            if balance > peak_balance {
                peak_balance = balance;
            }
            let dd = (peak_balance - balance) / peak_balance;
            if dd > max_dd {
                max_dd = dd;
            }
        }

        // Close any open position at last price
        if in_position && !data.is_empty() {
            let last = data.last().unwrap();
            let trade_pnl = (last.current_price - entry_price) * (entry_balance / entry_price);
            total_pnl += trade_pnl;
            trades += 1;
            if trade_pnl > 0.0 {
                wins += 1;
            }
        }

        // Sharpe ratio
        let sharpe = if returns.len() > 1 {
            let mean = returns.iter().sum::<f64>() / returns.len() as f64;
            let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
                / (returns.len() - 1) as f64;
            if variance > 0.0 {
                (mean / variance.sqrt()) * (252.0_f64).sqrt()
            } else {
                0.0
            }
        } else {
            0.0
        };

        BacktestResult {
            total_trades: trades as usize,
            win_rate: if trades > 0 {
                wins as f64 / trades as f64
            } else {
                0.0
            },
            total_pnl,
            max_drawdown: max_dd,
            sharpe_ratio: sharpe,
            decisions: decisions.into_iter().take(50).collect(),
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
    pub decisions: Vec<String>,
}
