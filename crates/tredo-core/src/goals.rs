use serde::{Deserialize, Serialize};

/// Agent trading mode — adjusts risk and behavior based on performance
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingMode {
    /// Maximize returns — wider stops, larger positions, more trades
    Aggressive,
    /// Standard risk parameters
    Normal,
    /// Preserve capital — tighter stops, smaller positions, fewer trades
    Conservative,
    /// No trading allowed — only manage existing positions
    Halted,
}

impl TradingMode {
    pub fn risk_multiplier(&self) -> f64 {
        match self {
            TradingMode::Aggressive => 1.5,
            TradingMode::Normal => 1.0,
            TradingMode::Conservative => 0.5,
            TradingMode::Halted => 0.0,
        }
    }

    pub fn min_confluence_bonus(&self) -> f64 {
        match self {
            TradingMode::Aggressive => 0.0, // lower threshold
            TradingMode::Normal => 0.05,
            TradingMode::Conservative => 0.10, // higher threshold
            TradingMode::Halted => 1.0,        // impossible
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            TradingMode::Aggressive => "Aggressive — seeking opportunities with wider parameters",
            TradingMode::Normal => "Normal — standard risk management",
            TradingMode::Conservative => "Conservative — capital preservation priority",
            TradingMode::Halted => "HALTED — no new trades permitted",
        }
    }
}

/// Daily and weekly trading goals that guide agent behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingGoals {
    /// Daily P&L target as a fraction of total equity (e.g., 0.005 = 0.5%)
    pub daily_target_pnl_pct: f64,
    /// Maximum number of trades per day
    pub max_daily_trades: u32,
    /// Maximum daily drawdown target (stricter than the hard limit)
    pub max_daily_drawdown_pct: f64,
    /// Minimum confidence score for entering a trade
    pub min_confidence_threshold: f64,
    /// Current behavior mode
    pub mode: TradingMode,
    /// Whether the daily target has been reached
    pub daily_target_reached: bool,
    /// Whether to stop trading after reaching daily target
    pub stop_after_target: bool,
}

impl Default for TradingGoals {
    fn default() -> Self {
        Self {
            daily_target_pnl_pct: 0.005, // 0.5% per day
            max_daily_trades: 10,
            max_daily_drawdown_pct: 0.02, // 2% (hard limit is 3%)
            min_confidence_threshold: 0.65,
            mode: TradingMode::Normal,
            daily_target_reached: false,
            stop_after_target: true,
        }
    }
}

impl TradingGoals {
    /// Re-evaluate the trading mode based on current portfolio state
    pub fn recalculate_mode(
        &mut self,
        daily_pnl_pct: f64,
        consecutive_losses: u32,
        total_trades_today: u32,
    ) {
        self.daily_target_reached = daily_pnl_pct >= self.daily_target_pnl_pct;

        if self.daily_target_reached && self.stop_after_target {
            self.mode = TradingMode::Conservative;
        } else if consecutive_losses >= 3 {
            self.mode = TradingMode::Halted;
        } else if consecutive_losses >= 2 || daily_pnl_pct < -self.max_daily_drawdown_pct {
            self.mode = TradingMode::Conservative;
        } else if total_trades_today < 3 && daily_pnl_pct >= 0.0 {
            self.mode = TradingMode::Aggressive;
        } else {
            self.mode = TradingMode::Normal;
        }
    }

    /// Get the effective minimum confidence for the current mode
    pub fn effective_min_confidence(&self) -> f64 {
        self.min_confidence_threshold + self.mode.min_confluence_bonus()
    }

    /// Get the effective max risk per trade based on mode
    pub fn effective_risk_multiplier(&self) -> f64 {
        self.mode.risk_multiplier()
    }
}
