//! # CircuitBreaker — Anomaly Detection & Auto-Halt for Live Trading
//!
//! Monitors several risk indicators in real-time and automatically halts live
//! trading when any threshold is breached. Provides graceful recovery with
//! configurable cool-down periods.
//!
//! ## Thresholds
//! - **Consecutive Rejections**: 3 → halt
//! - **Excessive Slippage**: 0.5% deviation from expected fill → halt
//! - **Connection Drops**: 5 consecutive → halt
//! - **Daily P&L Drop**: 5% from day's peak → halt
//! - **Rapid Order Rate**: 10+ orders in 60s with >50% failure → halt
//!
//! ## Recovery
//! After a halt, the breaker enters `Halted` state with a configurable cool-down.
//! After the cool-down expires, it returns to `Armed` — the system can auto-resume.

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

// ── Circuit Breaker States ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Normal operation — no issues detected
    Armed,
    /// Threshold breached — all live trading halted
    Halted,
    /// Cool-down period — waiting to auto-resume
    Recovery,
}

impl std::fmt::Display for BreakerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BreakerState::Armed => write!(f, "ARMED"),
            BreakerState::Halted => write!(f, "HALTED"),
            BreakerState::Recovery => write!(f, "RECOVERY"),
        }
    }
}

// ── Trigger Reason ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum HaltReason {
    ConsecutiveRejections(u32),
    ExcessiveSlippage(f64),
    ConnectionDrops(u32),
    DailyPnlDrop(f64),
    RapidOrderFailures(u32),
    ManualOverride(String),
}

impl std::fmt::Display for HaltReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HaltReason::ConsecutiveRejections(n) => write!(f, "{} consecutive order rejections", n),
            HaltReason::ExcessiveSlippage(pct) => write!(f, "Excessive slippage of {:.2}%", pct),
            HaltReason::ConnectionDrops(n) => write!(f, "{} consecutive connection drops", n),
            HaltReason::DailyPnlDrop(pct) => write!(f, "Daily P&L dropped {:.2}% from peak", pct),
            HaltReason::RapidOrderFailures(n) => write!(f, "{} order failures in 60s window", n),
            HaltReason::ManualOverride(msg) => write!(f, "Manual override: {}", msg),
        }
    }
}

// ── Configuration ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Max consecutive order rejections before halt
    pub max_consecutive_rejections: u32,
    /// Max allowable slippage (as % of expected fill price)
    pub max_slippage_pct: f64,
    /// Max consecutive connection drops before halt
    pub max_connection_drops: u32,
    /// Max allowable daily P&L drop from day's peak (as %)
    pub max_daily_pnl_drop_pct: f64,
    /// Max order failures in a 60-second window
    pub max_failures_per_minute: u32,
    /// Cool-down period in seconds before auto-resume
    pub cool_down_secs: u64,
    /// Minimum time between halts (seconds) — prevents rapid cycle
    pub min_halt_interval_secs: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            max_consecutive_rejections: 3,
            max_slippage_pct: 0.5,
            max_connection_drops: 5,
            max_daily_pnl_drop_pct: 5.0,
            max_failures_per_minute: 8,
            cool_down_secs: 300,         // 5 minutes
            min_halt_interval_secs: 600, // 10 minutes between halts
        }
    }
}

// ── Sliding Window for Failure Rate ──────────────────────────────────────────

#[derive(Debug, Clone)]
struct TimeStampedEvent {
    timestamp: DateTime<Utc>,
    is_failure: bool,
}

// ── CircuitBreaker ────────────────────────────────────────────────────────────

pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<BreakerState>,
    halt_reason: RwLock<Option<HaltReason>>,
    halted_at: RwLock<Option<DateTime<Utc>>>,
    last_halt_at: RwLock<Option<DateTime<Utc>>>,
    daily_peak_equity: RwLock<Option<f64>>,
    connection_drop_count: RwLock<u32>,
    recent_events: RwLock<Vec<TimeStampedEvent>>,
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("config", &self.config)
            .field("state", &self.state.try_read())
            .finish()
    }
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(BreakerState::Armed),
            halt_reason: RwLock::new(None),
            halted_at: RwLock::new(None),
            last_halt_at: RwLock::new(None),
            daily_peak_equity: RwLock::new(None),
            connection_drop_count: RwLock::new(0),
            recent_events: RwLock::new(Vec::new()),
        }
    }

    // ── State Queries ──────────────────────────────────────────────────────

    /// Returns the current breaker state
    pub async fn current_state(&self) -> BreakerState {
        *self.state.read().await
    }

    /// Returns whether live trading is permitted.
    /// If halted, checks if cool-down expired and auto-transitions to Recovery → Armed.
    pub async fn is_trading_allowed(&self) -> bool {
        let mut state = self.state.write().await;
        match *state {
            BreakerState::Armed => true,
            BreakerState::Recovery => {
                // Check if cool-down has expired → auto-resume
                let halted_at_val = *self.halted_at.read().await;
                if let Some(halted_at) = halted_at_val {
                    let elapsed = (Utc::now() - halted_at).num_seconds() as u64;
                    if elapsed >= self.config.cool_down_secs {
                        *state = BreakerState::Armed;
                        let mut reason = self.halt_reason.write().await;
                        *reason = None;
                        let mut halted = self.halted_at.write().await;
                        *halted = None;
                        println!(
                            "[CircuitBreaker] ✅ Auto-resumed after {}s cool-down",
                            elapsed
                        );
                        return true;
                    }
                }
                false
            }
            BreakerState::Halted => {
                // Check if cool-down expired → auto-resume directly to Armed
                let halted_at_val = *self.halted_at.read().await;
                if let Some(halted_at) = halted_at_val {
                    let elapsed = (Utc::now() - halted_at).num_seconds() as u64;
                    if elapsed >= self.config.cool_down_secs {
                        *state = BreakerState::Armed;
                        *self.halt_reason.write().await = None;
                        *self.halted_at.write().await = None;
                        println!(
                            "[CircuitBreaker] ✅ Cool-down expired — auto-resumed from Halted"
                        );
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Returns the halt reason if currently halted
    pub async fn halt_reason(&self) -> Option<HaltReason> {
        self.halt_reason.read().await.clone()
    }

    /// Returns time remaining in cool-down (seconds), if halted
    pub async fn cool_down_remaining(&self) -> Option<u64> {
        match *self.state.read().await {
            BreakerState::Recovery | BreakerState::Halted => {
                if let Some(halted_at) = *self.halted_at.read().await {
                    let elapsed = (Utc::now() - halted_at).num_seconds() as u64;
                    if elapsed < self.config.cool_down_secs {
                        return Some(self.config.cool_down_secs - elapsed);
                    }
                }
                None
            }
            BreakerState::Armed => None,
        }
    }

    // ── Monitoring Methods ─────────────────────────────────────────────────

    /// Report a successful order fill (resets connection drop counter, updates stats)
    pub async fn report_fill(&self) {
        let mut drops = self.connection_drop_count.write().await;
        *drops = 0;

        // Record success event
        let mut events = self.recent_events.write().await;
        events.push(TimeStampedEvent {
            timestamp: Utc::now(),
            is_failure: false,
        });
    }

    /// Report an order rejection — checks threshold
    pub async fn report_rejection(&self, consecutive_rejections: u32) -> Option<HaltReason> {
        if consecutive_rejections >= self.config.max_consecutive_rejections {
            let reason = HaltReason::ConsecutiveRejections(consecutive_rejections);
            self.trigger_halt(reason.clone()).await;
            return Some(reason);
        }

        // Record failure event
        {
            let mut events = self.recent_events.write().await;
            events.push(TimeStampedEvent {
                timestamp: Utc::now(),
                is_failure: true,
            });
        }

        // Check rapid failure rate
        self.check_rapid_failure_rate().await;

        None
    }

    /// Report excessive slippage — checks threshold
    pub async fn report_slippage(
        &self,
        expected_price: f64,
        actual_fill_price: f64,
    ) -> Option<HaltReason> {
        if expected_price <= 0.0 {
            return None;
        }
        let slippage_pct = ((actual_fill_price - expected_price) / expected_price).abs() * 100.0;
        if slippage_pct > self.config.max_slippage_pct {
            let reason = HaltReason::ExcessiveSlippage(slippage_pct);
            self.trigger_halt(reason.clone()).await;
            return Some(reason);
        }
        None
    }

    /// Report a connection drop — checks threshold
    pub async fn report_connection_drop(&self) -> Option<HaltReason> {
        let mut drops = self.connection_drop_count.write().await;
        *drops += 1;
        let count = *drops;
        drop(drops);

        if count >= self.config.max_connection_drops {
            let reason = HaltReason::ConnectionDrops(count);
            self.trigger_halt(reason.clone()).await;
            return Some(reason);
        }

        None
    }

    /// Update daily peak equity for P&L drawdown monitoring
    pub async fn update_equity(&self, current_equity: f64) -> Option<HaltReason> {
        let mut peak = self.daily_peak_equity.write().await;
        match *peak {
            Some(p) if current_equity > p => {
                *peak = Some(current_equity);
            }
            None => {
                *peak = Some(current_equity);
            }
            Some(p) => {
                let drop_pct = ((p - current_equity) / p) * 100.0;
                if drop_pct > self.config.max_daily_pnl_drop_pct {
                    let reason = HaltReason::DailyPnlDrop(drop_pct);
                    self.trigger_halt(reason.clone()).await;
                    return Some(reason);
                }
            }
        }
        None
    }

    // ── Manual Control ─────────────────────────────────────────────────────

    /// Manually halt trading with a reason (e.g., from UI or compliance)
    pub async fn manual_halt(&self, reason: &str) {
        self.trigger_halt(HaltReason::ManualOverride(reason.to_string()))
            .await;
    }

    /// Manually reset the breaker back to Armed
    pub async fn manual_reset(&self) {
        let mut state = self.state.write().await;
        *state = BreakerState::Armed;
        let mut reason = self.halt_reason.write().await;
        *reason = None;
        let mut halted_at = self.halted_at.write().await;
        *halted_at = None;
        let mut drops = self.connection_drop_count.write().await;
        *drops = 0;
        let mut events = self.recent_events.write().await;
        events.clear();
        let mut peak = self.daily_peak_equity.write().await;
        *peak = None;

        println!("[CircuitBreaker] 🔄 Manual reset — trading resumed");
    }

    /// Reset daily stats (call at start of each trading day)
    pub async fn reset_daily(&self) {
        let mut peak = self.daily_peak_equity.write().await;
        *peak = None;
        let mut events = self.recent_events.write().await;
        events.clear();
        println!("[CircuitBreaker] 📅 Daily stats reset");
    }

    // ── Internal Methods ───────────────────────────────────────────────────

    async fn trigger_halt(&self, reason: HaltReason) {
        let now = Utc::now();
        let mut state = self.state.write().await;

        // Don't re-trigger if already halted
        if *state != BreakerState::Armed {
            return;
        }

        // Check minimum halt interval
        let last_halt_val = *self.last_halt_at.read().await;
        if let Some(last_halt) = last_halt_val {
            let since_last_halt = (now - last_halt).num_seconds() as u64;
            if since_last_halt < self.config.min_halt_interval_secs {
                println!(
                    "[CircuitBreaker] ⏸ Skipping halt — only {}s since last halt (min: {}s)",
                    since_last_halt, self.config.min_halt_interval_secs
                );
                return;
            }
        }

        *state = BreakerState::Halted;
        let mut reason_lock = self.halt_reason.write().await;
        *reason_lock = Some(reason.clone());
        let mut halted_at = self.halted_at.write().await;
        *halted_at = Some(now);
        let mut last_halt = self.last_halt_at.write().await;
        *last_halt = Some(now);

        println!(
            "[CircuitBreaker] 🚨 CIRCUIT BREAKER TRIGGERED: {} at {}",
            reason,
            now.to_rfc3339()
        );
        println!(
            "[CircuitBreaker]    All live trading halted. Auto-resume in {}s (cool-down).",
            self.config.cool_down_secs
        );

        // State transitions are handled lazily by is_trading_allowed().
        // When the cool-down expires, the next call to is_trading_allowed()
        // will transition Halted → Recovery → Armed and auto-resume.
    }

    /// Check rapid failure rate within the last 60 seconds
    async fn check_rapid_failure_rate(&self) {
        let now = Utc::now();
        let mut events = self.recent_events.write().await;

        // Prune events older than 60 seconds
        events.retain(|e| (now - e.timestamp).num_seconds() < 60);

        let recent_failures = events.iter().filter(|e| e.is_failure).count() as u32;
        let total_recent = events.len() as u32;

        if total_recent >= 10 && recent_failures >= self.config.max_failures_per_minute {
            let reason = HaltReason::RapidOrderFailures(recent_failures);
            self.trigger_halt(reason).await;
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_initial_state_armed() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert_eq!(cb.current_state().await, BreakerState::Armed);
        assert!(cb.is_trading_allowed().await);
    }

    #[tokio::test]
    async fn test_rejection_threshold_triggers_halt() {
        let config = CircuitBreakerConfig {
            max_consecutive_rejections: 3,
            cool_down_secs: 1,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // 3 consecutive rejections should trigger halt
        for i in 1..=3 {
            let reason = cb.report_rejection(i).await;
            if i < 3 {
                assert!(reason.is_none(), "Should not halt at {} rejections", i);
                assert!(cb.is_trading_allowed().await);
            } else {
                assert!(reason.is_some(), "Should halt at {} rejections", i);
            }
        }

        assert!(!cb.is_trading_allowed().await);
        assert_eq!(cb.current_state().await, BreakerState::Halted);
    }

    #[tokio::test]
    async fn test_slippage_threshold() {
        let config = CircuitBreakerConfig {
            max_slippage_pct: 1.0,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // 0.5% slippage — should pass
        let reason = cb.report_slippage(100.0, 100.5).await;
        assert!(reason.is_none());
        assert!(cb.is_trading_allowed().await);

        // 2% slippage — should halt
        let reason = cb.report_slippage(100.0, 102.0).await;
        assert!(reason.is_some());
        assert!(!cb.is_trading_allowed().await);
    }

    #[tokio::test]
    async fn test_connection_drop_threshold() {
        let config = CircuitBreakerConfig {
            max_connection_drops: 3,
            cool_down_secs: 1,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        for i in 1..=3 {
            let reason = cb.report_connection_drop().await;
            if i < 3 {
                assert!(reason.is_none());
            } else {
                assert!(reason.is_some());
            }
        }

        assert!(!cb.is_trading_allowed().await);
    }

    #[tokio::test]
    async fn test_equity_drawdown() {
        let config = CircuitBreakerConfig {
            max_daily_pnl_drop_pct: 5.0,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // Set peak at 100000
        let reason = cb.update_equity(100000.0).await;
        assert!(reason.is_none());

        // Small drop — should pass
        let reason = cb.update_equity(98000.0).await;
        assert!(reason.is_none());

        // Large drop (6%) — should halt
        let reason = cb.update_equity(94000.0).await;
        assert!(reason.is_some());
        assert!(!cb.is_trading_allowed().await);
    }

    #[tokio::test]
    async fn test_manual_halt_and_reset() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());

        cb.manual_halt("Testing manual halt").await;
        assert_eq!(cb.current_state().await, BreakerState::Halted);
        assert!(!cb.is_trading_allowed().await);

        let reason = cb.halt_reason().await;
        assert_eq!(
            reason,
            Some(HaltReason::ManualOverride(
                "Testing manual halt".to_string()
            ))
        );

        cb.manual_reset().await;
        assert_eq!(cb.current_state().await, BreakerState::Armed);
        assert!(cb.is_trading_allowed().await);
    }

    #[tokio::test]
    async fn test_reset_daily() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        cb.update_equity(100000.0).await;
        cb.report_fill().await;
        cb.report_connection_drop().await;

        cb.reset_daily().await;
        assert!(cb.is_trading_allowed().await);
    }

    #[tokio::test]
    async fn test_cool_down_expiry() {
        let config = CircuitBreakerConfig {
            max_consecutive_rejections: 1,
            cool_down_secs: 0, // Immediate recovery
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        cb.report_rejection(1).await;
        assert_eq!(cb.current_state().await, BreakerState::Halted);

        // Wait a tiny bit for the recovery task
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Should now be in Recovery and is_trading_allowed will auto-resume
        // Actually, with cool_down_secs=0, it transitions to Recovery immediately
        // but is_trading_allowed checks the elapsed time and returns true for Recovery
        assert!(
            cb.is_trading_allowed().await,
            "Should auto-resume after cool-down"
        );
    }

    #[tokio::test]
    async fn test_fill_resets_connection_drops() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());

        cb.report_connection_drop().await;
        cb.report_connection_drop().await;
        cb.report_fill().await; // Should reset

        // Should still be armed after one more drop
        let reason = cb.report_connection_drop().await;
        assert!(reason.is_none(), "Connection drops should have been reset");

        let config = CircuitBreakerConfig {
            max_connection_drops: 1,
            cool_down_secs: 1,
            ..Default::default()
        };
        let cb2 = CircuitBreaker::new(config);
        let reason = cb2.report_connection_drop().await;
        assert!(reason.is_some(), "Should halt after 1 drop with max=1");
    }
}
