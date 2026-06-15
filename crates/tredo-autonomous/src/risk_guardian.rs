use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskGuardianConfig {
    pub max_risk_per_trade: f64,
    pub max_daily_drawdown: f64,
    pub consecutive_loss_limit: u32,
    // Fields used by EvolvedMetaControl for adaptation (from user spec)
    pub max_risk_per_trade_pct: f64,
    pub absolute_max_leverage: u32,
}

impl RiskGuardianConfig {
    pub fn default_fallback() -> Self {
        Self {
            max_risk_per_trade: 0.01,
            max_daily_drawdown: 0.03,
            consecutive_loss_limit: 3,
            max_risk_per_trade_pct: 0.01,
            absolute_max_leverage: 3,
        }
    }
}