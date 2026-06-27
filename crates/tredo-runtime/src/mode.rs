use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
pub enum TradingMode {
    Paper,
    Live,
    Backtest,
    Validate,
    Research,
}

impl fmt::Display for TradingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paper => write!(f, "paper"),
            Self::Live => write!(f, "live"),
            Self::Backtest => write!(f, "backtest"),
            Self::Validate => write!(f, "validate"),
            Self::Research => write!(f, "research"),
        }
    }
}

impl std::str::FromStr for TradingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "paper" => Ok(Self::Paper),
            "live" => Ok(Self::Live),
            "backtest" => Ok(Self::Backtest),
            "validate" => Ok(Self::Validate),
            "research" => Ok(Self::Research),
            _ => Err(format!(
                "Unknown mode: {}. Use: paper|live|backtest|validate|research",
                s
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModeConfig {
    pub mode: TradingMode,
    pub require_trade_confirmation: bool,
    pub max_daily_loss: f64,
    pub symbol_whitelist: Option<Vec<String>>,
    pub backtest_start: Option<chrono::DateTime<chrono::Utc>>,
    pub backtest_end: Option<chrono::DateTime<chrono::Utc>>,
    pub backtest_data_path: Option<String>,
    pub backtest_initial_capital: f64,
    pub validate_cycles: usize,
    pub induce_regret: bool,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            mode: TradingMode::Paper,
            require_trade_confirmation: true,
            max_daily_loss: 1000.0,
            symbol_whitelist: None,
            backtest_start: None,
            backtest_end: None,
            backtest_data_path: None,
            backtest_initial_capital: 100_000.0,
            validate_cycles: 50,
            induce_regret: false,
        }
    }
}
