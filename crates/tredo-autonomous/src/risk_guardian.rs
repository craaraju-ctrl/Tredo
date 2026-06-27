use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskGuardianConfig {
    pub max_risk_per_trade: f64,
    pub max_daily_drawdown: f64,
    pub consecutive_loss_limit: u32,
    pub max_risk_per_trade_pct: f64,
    pub absolute_max_leverage: u32,
    pub absolute_max_drawdown_pct: f64,
    pub hard_min_stop_loss_pct: f64,
    pub hard_max_stop_loss_pct: f64,
}

impl RiskGuardianConfig {
    pub fn default_fallback() -> Self {
        Self {
            max_risk_per_trade: 0.01,
            max_daily_drawdown: 0.03,
            consecutive_loss_limit: 3,
            max_risk_per_trade_pct: 0.02,
            absolute_max_leverage: 3,
            absolute_max_drawdown_pct: 0.15,
            hard_min_stop_loss_pct: 0.005,
            hard_max_stop_loss_pct: 0.08,
        }
    }
}

/// A proposed trade awaiting risk validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedTrade {
    pub symbol: String,
    pub entry_price: f64,
    pub stop_loss_price: f64,
    pub position_size: f64,
    pub leverage: u32,
}

/// Portfolio context snapshot for risk checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianPortfolioContext {
    pub current_drawdown_pct: f64,
    pub total_equity: f64,
}

/// Compiled risk firewall that validates proposed trades against hardcoded safety limits.
#[derive(Debug, Clone)]
pub struct RiskGuardian {
    pub config: RiskGuardianConfig,
}

impl RiskGuardian {
    pub fn new(config: RiskGuardianConfig) -> Self {
        Self { config }
    }

    /// Intercepts and validates a proposed trade. Returns Ok(()) if safe, Err with violation reason otherwise.
    pub fn intercept_and_validate(
        &self,
        proposed: &ProposedTrade,
        context: &GuardianPortfolioContext,
    ) -> Result<(), String> {
        if context.current_drawdown_pct > self.config.absolute_max_drawdown_pct {
            return Err(format!(
                "Max drawdown exceeded: {:.2}% > {:.2}%",
                context.current_drawdown_pct * 100.0,
                self.config.absolute_max_drawdown_pct * 100.0
            ));
        }
        if proposed.leverage > self.config.absolute_max_leverage {
            return Err(format!(
                "Leverage {} exceeds max {}",
                proposed.leverage, self.config.absolute_max_leverage
            ));
        }
        if proposed.position_size <= 0.0 {
            return Err("Position size must be positive".to_string());
        }

        // === NEW: Enforce hard SL bounds ===
        if proposed.stop_loss_price > 0.0 && proposed.entry_price > 0.0 {
            let stop_pct = ((proposed.entry_price - proposed.stop_loss_price).abs()
                / proposed.entry_price)
                * 100.0;
            if stop_pct < self.config.hard_min_stop_loss_pct * 100.0 {
                return Err(format!(
                    "Stop loss too tight: {:.3}% < min {:.3}% (prevents noise-driven exits)",
                    stop_pct,
                    self.config.hard_min_stop_loss_pct * 100.0
                ));
            }
            if stop_pct > self.config.hard_max_stop_loss_pct * 100.0 {
                return Err(format!(
                    "Stop loss too wide: {:.3}% > max {:.3}% (prevents excessive risk per trade)",
                    stop_pct,
                    self.config.hard_max_stop_loss_pct * 100.0
                ));
            }
        }

        Ok(())
    }
}
