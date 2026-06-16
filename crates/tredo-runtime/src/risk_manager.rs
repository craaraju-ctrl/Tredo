//! Centralized risk manager — per-trade limits, portfolio-level circuit breakers,
//! and the safety gates that prevent the agent from self-destructing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Why a trade was rejected by the risk manager.
#[derive(Debug, Clone)]
pub enum RiskRejection {
    MaxDailyLossExceeded { daily_loss: f64, max_loss: f64 },
    MaxDrawdownExceeded { current_dd: f64, max_dd: f64 },
    MaxPositionSizeExceeded { proposed: f64, max_allowed: f64 },
    PortfolioHeatExceeded { heat: f64, max_heat: f64 },
    ConsecutiveLossLimitHit { losses: u32, max_losses: u32 },
    CircuitBreakerTriggered { reason: String },
    SymbolBlacklisted { symbol: String },
    InsufficientData { missing: String },
    ConfidenceTooLow { confidence: f64, threshold: f64 },
}

impl std::fmt::Display for RiskRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxDailyLossExceeded { daily_loss, max_loss } => {
                write!(f, "Daily loss ₹{:.2} exceeds max ₹{:.2}", daily_loss, max_loss)
            }
            Self::MaxDrawdownExceeded { current_dd, max_dd } => {
                write!(f, "Drawdown {:.1}% exceeds max {:.1}%", current_dd * 100.0, max_dd * 100.0)
            }
            Self::MaxPositionSizeExceeded { proposed, max_allowed } => {
                write!(f, "Position size ₹{:.2} > max ₹{:.2}", proposed, max_allowed)
            }
            Self::PortfolioHeatExceeded { heat, max_heat } => {
                write!(f, "Portfolio heat {:.1}% > max {:.1}%", heat * 100.0, max_heat * 100.0)
            }
            Self::ConsecutiveLossLimitHit { losses, max_losses } => {
                write!(f, "{} consecutive losses (max {})", losses, max_losses)
            }
            Self::CircuitBreakerTriggered { reason } => {
                write!(f, "Circuit breaker: {}", reason)
            }
            Self::SymbolBlacklisted { symbol } => {
                write!(f, "Symbol {} is blacklisted", symbol)
            }
            Self::InsufficientData { missing } => {
                write!(f, "Insufficient data: {}", missing)
            }
            Self::ConfidenceTooLow { confidence, threshold } => {
                write!(f, "Confidence {:.2} < threshold {:.2}", confidence, threshold)
            }
        }
    }
}

impl std::error::Error for RiskRejection {}

/// Aggregate result of a risk check.
#[derive(Debug, Clone)]
pub enum RiskDecision {
    /// Trade is approved.
    Approved { reason: String },
    /// Trade is approved with warnings.
    ApprovedWithCaution { warnings: Vec<String> },
    /// Trade is rejected.
    Rejected { reason: RiskRejection },
}

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// Tracks daily loss/win aggregated state.
#[derive(Debug, Clone)]
pub struct DailyRiskState {
    pub date: String,
    pub realized_pnl: f64,
    pub trade_count: u32,
    pub loss_count: u32,
    pub consecutive_losses: u32,
    pub max_favorable_excursion: f64,
    pub max_adverse_excursion: f64,
}

/// Centralized risk manager.
pub struct RiskManager {
    /// Circuit breaker state per reason.
    breakers: parking_lot::RwLock<std::collections::HashMap<String, BreakerState>>,
    /// Today's aggregated risk state.
    daily: Arc<parking_lot::RwLock<DailyRiskState>>,
    /// Whether a hard stop is engaged (e.g., max drawdown hit).
    hard_stop_engaged: AtomicBool,
    /// Counter for rapid-fire order rejection (DoS protection).
    rapid_order_counter: AtomicU64,
}

impl Default for RiskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskManager {
    pub fn new() -> Self {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        Self {
            breakers: parking_lot::RwLock::new(std::collections::HashMap::new()),
            daily: Arc::new(parking_lot::RwLock::new(DailyRiskState {
                date: today,
                realized_pnl: 0.0,
                trade_count: 0,
                loss_count: 0,
                consecutive_losses: 0,
                max_favorable_excursion: 0.0,
                max_adverse_excursion: 0.0,
            })),
            hard_stop_engaged: AtomicBool::new(false),
            rapid_order_counter: AtomicU64::new(0),
        }
    }

    /// Check a proposed trade against all risk limits.
    pub async fn check_trade(
        &self,
        symbol: &str,
        position_size: f64,
        total_equity: f64,
        _current_pnl: f64,
        confidence: f64,
    ) -> RiskDecision {
        // Hard stop check
        if self.hard_stop_engaged.load(Ordering::Relaxed) {
            return RiskDecision::Rejected {
                reason: RiskRejection::CircuitBreakerTriggered {
                    reason: "Hard stop engaged — max drawdown exceeded".into(),
                },
            };
        }

        // Rapid-fire protection
        let count = self.rapid_order_counter.fetch_add(1, Ordering::Relaxed);
        if count > 10 {
            return RiskDecision::Rejected {
                reason: RiskRejection::CircuitBreakerTriggered {
                    reason: "Too many rapid trade attempts".into(),
                },
            };
        }

        let daily = self.daily.read();

        // Max daily loss check
        let max_daily_loss = total_equity * 0.05; // 5% daily max
        if daily.realized_pnl < -max_daily_loss {
            return RiskDecision::Rejected {
                reason: RiskRejection::MaxDailyLossExceeded {
                    daily_loss: daily.realized_pnl.abs(),
                    max_loss: max_daily_loss,
                },
            };
        }

        // Consecutive loss check
        if daily.consecutive_losses >= 5 {
            return RiskDecision::Rejected {
                reason: RiskRejection::ConsecutiveLossLimitHit {
                    losses: daily.consecutive_losses,
                    max_losses: 5,
                },
            };
        }

        // Position size check (max 20% of equity per trade)
        let max_position = total_equity * 0.20;
        if position_size > max_position {
            return RiskDecision::Rejected {
                reason: RiskRejection::MaxPositionSizeExceeded {
                    proposed: position_size,
                    max_allowed: max_position,
                },
            };
        }

        // Confidence check
        let confidence_threshold = match daily.consecutive_losses {
            0..=1 => 0.30,
            2..=3 => 0.50,
            _ => 0.65,
        };
        if confidence < confidence_threshold {
            return RiskDecision::Rejected {
                reason: RiskRejection::ConfidenceTooLow {
                    confidence,
                    threshold: confidence_threshold,
                },
            };
        }

        drop(daily);

        RiskDecision::Approved {
            reason: format!("All risk checks passed for {}", symbol),
        }
    }

    /// Record a trade outcome (win/loss) for daily aggregation.
    pub fn record_trade_outcome(&self, pnl: f64) {
        let mut daily = self.daily.write();
        daily.trade_count += 1;
        daily.realized_pnl += pnl;
        if pnl < 0.0 {
            daily.loss_count += 1;
            daily.consecutive_losses += 1;
        } else {
            daily.consecutive_losses = 0;
        }
        if pnl > daily.max_favorable_excursion {
            daily.max_favorable_excursion = pnl;
        }
        if pnl < daily.max_adverse_excursion {
            daily.max_adverse_excursion = pnl;
        }
        // Reset rapid order counter on successful trades
        self.rapid_order_counter.store(0, Ordering::Relaxed);
    }

    /// Trigger a circuit breaker.
    pub fn trigger_breaker(&self, reason: &str) {
        let mut breakers = self.breakers.write();
        breakers.insert(reason.to_string(), BreakerState::Open);
        tracing::warn!("Circuit breaker triggered: {}", reason);
    }

    /// Check if specifc breaker is open.
    pub fn is_breaker_open(&self, reason: &str) -> bool {
        let breakers = self.breakers.read();
        breakers.get(reason) == Some(&BreakerState::Open)
    }

    /// Engage/disengage the hard stop.
    pub fn set_hard_stop(&self, engaged: bool) {
        self.hard_stop_engaged.store(engaged, Ordering::Relaxed);
        if engaged {
            tracing::error!("HARD STOP ENGAGED — all trading suspended");
        } else {
            tracing::info!("Hard stop released — trading resumed");
        }
    }

    /// Reset daily state (call at start of each trading day).
    pub fn reset_daily(&self) {
        let mut daily = self.daily.write();
        daily.date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        daily.realized_pnl = 0.0;
        daily.trade_count = 0;
        daily.loss_count = 0;
        daily.consecutive_losses = 0;
        daily.max_favorable_excursion = 0.0;
        daily.max_adverse_excursion = 0.0;
        self.rapid_order_counter.store(0, Ordering::Relaxed);
    }

    pub fn daily_state(&self) -> DailyRiskState {
        self.daily.read().clone()
    }

    pub fn is_hard_stop_engaged(&self) -> bool {
        self.hard_stop_engaged.load(Ordering::Relaxed)
    }

    /// Summary string for dashboard/logs.
    pub fn summary(&self) -> String {
        let d = self.daily.read();
        format!(
            "Daily {} | P&L ₹{:.2} | {} trades | {} consecutive losses | HardStop: {}",
            d.date, d.realized_pnl, d.trade_count, d.consecutive_losses,
            if self.hard_stop_engaged.load(Ordering::Relaxed) { "ENGAGED" } else { "OFF" }
        )
    }
}
