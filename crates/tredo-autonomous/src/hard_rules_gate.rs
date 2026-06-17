// ═══════════════════════════════════════════════════════════════════════════════
// Hard Rules Gate — Single top-level enforcement of ALL hard rules
//
// Architecture (per institutional best practice):
//
//   ┌─────────────────────────────────────────────────────────────────┐
//   │                    HARD RULES GATE (Layer 1)                    │
//   │  Priority: Critical > High > Medium > Low                      │
//   │  Override: Upper layer ALWAYS wins. Equal priority → conservative│
//   │  Blocking: Critical/High always block. Medium blocks only if    │
//   │            no Higher rule overrides. Low = WARNING only.        │
//   └─────────────────────────────────────────────────────────────────┘
//                              ↓ (if passed)
//   ┌─────────────────────────────────────────────────────────────────┐
//   │                 REGIME DETECTION (Layer 2)                      │
//   │  Dynamic thresholds based on market state                      │
//   └─────────────────────────────────────────────────────────────────┘
//                              ↓
//   ┌─────────────────────────────────────────────────────────────────┐
//   │               DEBATE LAYER (Layer 3) — Advisory Only           │
//   │  6 agents provide evidence + confidence. No veto power.        │
//   └─────────────────────────────────────────────────────────────────┘
//                              ↓
//   ┌─────────────────────────────────────────────────────────────────┐
//   │           JUDGE / ADJUDICATOR (Layer 4) — Final Authority      │
//   │  Combines hard rules + debate evidence → BUY/HOLD/SELL         │
//   └─────────────────────────────────────────────────────────────────┘
//                              ↓
//   ┌─────────────────────────────────────────────────────────────────┐
//   │              EXECUTION LAYER (Layer 5)                         │
//   │  Executes the adjudicated decision                             │
//   └─────────────────────────────────────────────────────────────────┘
//
// Key principle: Debate agents are ADVISORY only. Only the Judge has
// decision-making power. This prevents "hallucinated conviction" and
// ensures hard rules are never bypassed by agent enthusiasm.
// ═══════════════════════════════════════════════════════════════════════════════

use crate::state::SharedState;
use crate::types::{HardRulesGateResult, RuleCheck, RulePriority};
use chrono::Utc;

pub struct HardRulesGate {
    state: SharedState,
}

impl HardRulesGate {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Run ALL hard rules in priority order.
    /// Returns HardRulesGateResult with pass/fail and which rules failed.
    ///
    /// Priority-based blocking logic:
    /// - Critical/High failures: ALWAYS block (no override possible)
    /// - Medium failures: block ONLY if no Critical/High rule has already been checked
    ///   (i.e., Medium rules are soft-blocks that can be overridden by a Higher rule passing)
    /// - Low failures: WARNINGS ONLY — logged but never block
    ///
    /// This prevents a Low-priority position-size preference from blocking a
    /// Critical drawdown halt, while still enforcing Medium-priority regime checks.
    pub async fn evaluate(&self, symbol: &str) -> HardRulesGateResult {
        let mut failed_rules = Vec::new();
        let mut highest_blocking_priority = None; // Track highest priority that should block
        let mut total_checked = 0;

        // ── CRITICAL PRIORITY (Never overridden) ────────────────────────────
        // These rules stop trading immediately, regardless of any agent's opinion.

        // 1. Trading enabled check
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            if !portfolio.trading_enabled {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "trading_enabled".to_string(),
                    priority: RulePriority::Critical,
                    reason: "Trading is disabled (drawdown halt active)".to_string(),
                });
                if highest_blocking_priority.is_none() {
                    highest_blocking_priority = Some(RulePriority::Critical);
                }
            }
        }

        // 2. Daily drawdown limit (2% hard limit)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            let dd = portfolio.max_drawdown_today;
            if dd > 0.02 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "daily_drawdown".to_string(),
                    priority: RulePriority::Critical,
                    reason: format!("Daily drawdown at {:.2}% exceeds 2% hard limit", dd * 100.0),
                });
                if highest_blocking_priority.is_none() {
                    highest_blocking_priority = Some(RulePriority::Critical);
                }
            }
        }

        // 3. Red folder discipline
        total_checked += 1;
        {
            let rules = self.state.rules.read().await;
            if rules.red_folder_discipline {
                let calendar = self.state.calendar_events.read().await;
                let today_str = Utc::now().format("%Y-%m-%d").to_string();
                let has_red_folder = calendar.iter().any(|event| {
                    event.impact == tredo_core::calendar::EventImpact::High
                        && event.date == today_str
                });
                if has_red_folder {
                    failed_rules.push(RuleCheck {
                        passed: false,
                        rule_name: "red_folder".to_string(),
                        priority: RulePriority::Critical,
                        reason: "High-impact economic event today".to_string(),
                    });
                    if highest_blocking_priority.is_none() {
                        highest_blocking_priority = Some(RulePriority::Critical);
                    }
                }
            }
        }

        // 4. Session timing (crypto bypasses this)
        total_checked += 1;
        {
            let rules = self.state.rules.read().await;
            let is_crypto = matches!(symbol, "BTC" | "ETH" | "SOL");
            if !is_crypto && rules.respect_session_timing {
                let now = Utc::now();
                if !crate::helpers::get_indian_session_info(now).market_open {
                    // Also check London/NY sessions
                    if !tredo_core::is_in_trading_session(now, &rules) {
                        failed_rules.push(RuleCheck {
                            passed: false,
                            rule_name: "session_timing".to_string(),
                            priority: RulePriority::Critical,
                            reason: "Outside allowed trading sessions".to_string(),
                        });
                        if highest_blocking_priority.is_none() {
                            highest_blocking_priority = Some(RulePriority::Critical);
                        }
                    }
                }
            }
        }

        // ── HIGH PRIORITY (Risk limits, circuit breakers) ───────────────────

        // 5. Portfolio heat limit (10%)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            let heat = if portfolio.total_equity > 0.0 {
                portfolio.open_positions.iter().map(|p| p.risk_amount).sum::<f64>() / portfolio.total_equity
            } else {
                0.0
            };
            if heat > 0.10 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "portfolio_heat".to_string(),
                    priority: RulePriority::High,
                    reason: format!("Portfolio heat at {:.1}% exceeds 10% limit", heat * 100.0),
                });
                if highest_blocking_priority.is_none() || highest_blocking_priority == Some(RulePriority::Medium) {
                    highest_blocking_priority = Some(RulePriority::High);
                }
            }
        }

        // 6. Consecutive loss circuit breaker (4+)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            if portfolio.consecutive_losses >= 4 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "loss_circuit_breaker".to_string(),
                    priority: RulePriority::High,
                    reason: format!("{} consecutive losses — circuit breaker triggered", portfolio.consecutive_losses),
                });
                if highest_blocking_priority.is_none() || highest_blocking_priority == Some(RulePriority::Medium) {
                    highest_blocking_priority = Some(RulePriority::High);
                }
            }
        }

        // 7. Max daily trades (8 total)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            if portfolio.total_trades_today >= 8 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "max_daily_trades".to_string(),
                    priority: RulePriority::High,
                    reason: format!("{} trades today exceeds 8-trade daily limit", portfolio.total_trades_today),
                });
                if highest_blocking_priority.is_none() || highest_blocking_priority == Some(RulePriority::Medium) {
                    highest_blocking_priority = Some(RulePriority::High);
                }
            }
        }

        // 8. Cooldown check (30 min between same-symbol trades)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            if let Some(last_trade_time) = portfolio.last_trade_time {
                let elapsed = Utc::now() - last_trade_time;
                if elapsed.num_seconds() < 1800 {
                    failed_rules.push(RuleCheck {
                        passed: false,
                        rule_name: "cooldown".to_string(),
                        priority: RulePriority::High,
                        reason: format!("{}s since last trade on {} (min 1800s)", elapsed.num_seconds(), symbol),
                    });
                    if highest_blocking_priority.is_none() || highest_blocking_priority == Some(RulePriority::Medium) {
                        highest_blocking_priority = Some(RulePriority::High);
                    }
                }
            }
        }

        // ── MEDIUM PRIORITY (Regime, confluence) ────────────────────────────

        // 9. Regime safety: no BUY in bear regime with low confluence
        total_checked += 1;
        {
            let regime = *self.state.market_regime.read().await;
            if regime == Some(crate::types::MarketRegime::TrendingBear) {
                let confluence = {
                    let agg = self.state.last_aggregated_signal.read().await;
                    agg.as_ref().map(|a| a.conviction).unwrap_or(0.5)
                };
                if confluence < 0.60 {
                    failed_rules.push(RuleCheck {
                        passed: false,
                        rule_name: "regime_safety".to_string(),
                        priority: RulePriority::Medium,
                        reason: format!("Bear regime with confluence {:.1}% < 60% minimum", confluence * 100.0),
                    });
                    // Medium only blocks if no Critical/High already set
                    if highest_blocking_priority.is_none() {
                        highest_blocking_priority = Some(RulePriority::Medium);
                    }
                }
            }
        }

        // 10. Confluence minimum (regime-adaptive)
        total_checked += 1;
        {
            let confluence = {
                let agg = self.state.last_aggregated_signal.read().await;
                agg.as_ref().map(|a| a.conviction).unwrap_or(0.5)
            };
            let regime = *self.state.market_regime.read().await;
            let min_confluence = match &regime {
                Some(crate::types::MarketRegime::TrendingBull) => 0.50,
                Some(crate::types::MarketRegime::TrendingBear) => 0.80,
                Some(crate::types::MarketRegime::Ranging) => 0.70,
                Some(crate::types::MarketRegime::Volatile) => 0.75,
                Some(crate::types::MarketRegime::LowLiquidity) => 0.85,
                None => 0.65,
            };
            if confluence < min_confluence {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "confluence_minimum".to_string(),
                    priority: RulePriority::Medium,
                    reason: format!("Confluence {:.1}% below regime minimum {:.1}%", confluence * 100.0, min_confluence * 100.0),
                });
                if highest_blocking_priority.is_none() {
                    highest_blocking_priority = Some(RulePriority::Medium);
                }
            }
        }

        // ── LOW PRIORITY (Position limits — WARNINGS ONLY) ──────────────────

        // 11. Open position check (max 3 per symbol)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            let sym_positions = portfolio.open_positions.iter().filter(|p| p.symbol == symbol).count();
            if sym_positions >= 3 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "max_positions_per_symbol".to_string(),
                    priority: RulePriority::Low,
                    reason: format!("{} positions on {} — max 3 per symbol", sym_positions, symbol),
                });
                // Low priority: never blocks, just warns
            }
        }

        // 12. Max total open positions (10)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            if portfolio.open_positions.len() >= 10 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "max_total_positions".to_string(),
                    priority: RulePriority::Low,
                    reason: format!("{} open positions — max 10 total", portfolio.open_positions.len()),
                });
                // Low priority: never blocks, just warns
            }
        }

        // ── Determine pass/fail using priority-based blocking ────────────────
        // Critical/High always block. Medium blocks only if no Higher rule overrides.
        // Low never blocks.
        let passed = highest_blocking_priority.is_none()
            || highest_blocking_priority == Some(RulePriority::Low);

        // Log results
        if passed {
            if !failed_rules.is_empty() {
                println!(
                    "[HardRulesGate] ⚠️  {} warnings for {} (none blocking)",
                    failed_rules.len(), symbol
                );
                for rule in &failed_rules {
                    println!("  - [WARNING] {}: {}", rule.rule_name, rule.reason);
                }
            } else {
                println!("[HardRulesGate] ✅ All {} rules passed for {}", total_checked, symbol);
            }
        } else {
            println!(
                "[HardRulesGate] ⛔ {}/{} rules failed for {} (blocking priority: {:?})",
                failed_rules.len(),
                total_checked,
                symbol,
                highest_blocking_priority
            );
            for rule in &failed_rules {
                if rule.priority >= RulePriority::Medium {
                    println!("  - [BLOCK] [{:?}] {}: {}", rule.priority, rule.rule_name, rule.reason);
                } else {
                    println!("  - [WARN] [{:?}] {}: {}", rule.priority, rule.rule_name, rule.reason);
                }
            }
        }

        HardRulesGateResult {
            passed,
            failed_rules,
            highest_failed_priority: highest_blocking_priority,
            total_rules_checked: total_checked,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Unit Tests — Priority-based blocking logic
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MarketRegime, OpenPosition};
    use chrono::{Duration, Utc};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tredo_core::{Config, DisciplineRules, MemoryStore, TradeDirection};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Create a clean SharedState for testing. Uses unique DB names to avoid
    /// DatabaseAlreadyOpen when tests run in parallel.
    async fn setup_state() -> SharedState {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let redb_path = format!("file:test_hrg_{}.redb?mode=memory", id);
        let memory = MemoryStore::new(&redb_path).expect("MemoryStore creation");
        let config = Config {
            kronos_service_url: "http://127.0.0.1:19999".to_string(),
            ..Config::default()
        };
        let rules = DisciplineRules::default();
        SharedState::new(memory, rules, config, ":memory:").expect("SharedState init")
    }

    /// Create a SharedState with trading disabled (Critical rule).
    async fn setup_state_trading_disabled() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.trading_enabled = false;
        }
        state
    }

    /// Create a SharedState with drawdown > 2% (Critical rule).
    async fn setup_state_drawdown() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.max_drawdown_today = 0.025; // 2.5% > 2% limit
        }
        state
    }

    /// Create a SharedState with high portfolio heat (High rule).
    async fn setup_state_high_heat() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.total_equity = 100_000.0;
            // Add positions with total risk = 12% of equity
            for i in 0..3 {
                portfolio.open_positions.push(OpenPosition {
                    symbol: format!("SYM{}", i),
                    direction: TradeDirection::Long,
                    entry_price: 100.0,
                    current_price: 100.0,
                    stop_loss: 95.0,
                    take_profit: 110.0,
                    quantity: 1.0,
                    unrealized_pnl: 0.0,
                    unrealized_pnl_pct: 0.0,
                    entry_time: Utc::now(),
                    risk_amount: 4000.0, // 3 × 4000 = 12000 / 100000 = 12%
                });
            }
        }
        state
    }

    /// Create a SharedState with 4+ consecutive losses (High rule).
    async fn setup_state_consecutive_losses() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.consecutive_losses = 5;
        }
        state
    }

    /// Create a SharedState with 8+ daily trades (High rule).
    async fn setup_state_max_trades() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.total_trades_today = 9;
        }
        state
    }

    /// Create a SharedState with recent trade (cooldown active — High rule).
    async fn setup_state_cooldown() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.last_trade_time = Some(Utc::now() - Duration::seconds(300)); // 5 min ago < 30 min
        }
        state
    }

    /// Create a SharedState with bear regime + low confluence (Medium rule).
    async fn setup_state_bear_regime_low_confluence() -> SharedState {
        let state = setup_state().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::TrendingBear);
            // No aggregated signal → default confluence = 0.5 < 0.80 minimum for bear
        }
        state
    }

    /// Create a SharedState with no aggregated signal and unknown regime (Medium rule).
    /// Default confluence = 0.5, minimum for None regime = 0.65.

    /// Create a SharedState with high confluence (all Medium rules pass).
    async fn setup_state_high_confluence() -> SharedState {
        let state = setup_state().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::TrendingBull);
            // Seed high confluence so Medium rules pass
            let agg = tredo_core::skill_aggregator::AggregatedSignal {
                net_signal: 0.6,
                bullish_strength: 0.7,
                bearish_strength: 0.1,
                conviction: 0.85, // well above all minimums
                consensus: Some(tredo_core::agent::SkillDirection::Bullish),
                participating_count: 5,
                bullish_count: 4,
                bearish_count: 1,
                neutral_count: 0,
            };
            *state.last_aggregated_signal.write().await = Some(agg);
        }
        state
    }

    /// Create a SharedState with 3 positions on the same symbol (Low rule).
    async fn setup_state_max_positions_per_symbol() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            for i in 0..3 {
                portfolio.open_positions.push(OpenPosition {
                    symbol: "BTC".to_string(),
                    direction: TradeDirection::Long,
                    entry_price: 60000.0 + i as f64 * 1000.0,
                    current_price: 61000.0,
                    stop_loss: 58000.0,
                    take_profit: 65000.0,
                    quantity: 0.1,
                    unrealized_pnl: 0.0,
                    unrealized_pnl_pct: 0.0,
                    entry_time: Utc::now(),
                    risk_amount: 200.0, // each 200 / 100000 = 0.2%
                });
            }
            portfolio.total_equity = 100_000.0;
        }
        state
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: All rules pass → passed = true
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_all_rules_pass() {
        let state = setup_state_high_confluence().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(result.passed, "All rules should pass when state is clean");
        assert!(
            result.failed_rules.is_empty(),
            "No rules should fail when state is clean"
        );
        assert!(
            result.highest_failed_priority.is_none(),
            "No blocking priority when all pass"
        );
        assert_eq!(result.total_rules_checked, 12, "Should check all 12 rules");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Critical rule always blocks
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_critical_always_blocks() {
        // Scenario 1: Trading disabled (Critical)
        let state = setup_state_trading_disabled().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(!result.passed, "Trading disabled should block");
        assert_eq!(result.highest_failed_priority, Some(RulePriority::Critical));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "trading_enabled"));

        // Scenario 2: Drawdown > 2% (Critical)
        let state = setup_state_drawdown().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("ETH").await;

        assert!(!result.passed, "Drawdown > 2% should block");
        assert_eq!(result.highest_failed_priority, Some(RulePriority::Critical));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "daily_drawdown"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: High priority always blocks
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_high_always_blocks() {
        // Scenario 1: Portfolio heat > 10% (High)
        let state = setup_state_high_heat().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("SYM0").await;

        assert!(!result.passed, "High heat should block");
        assert_eq!(result.highest_failed_priority, Some(RulePriority::High));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "portfolio_heat"));

        // Scenario 2: 5 consecutive losses (High)
        let state = setup_state_consecutive_losses().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(!result.passed, "4+ consecutive losses should block");
        assert_eq!(result.highest_failed_priority, Some(RulePriority::High));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "loss_circuit_breaker"));

        // Scenario 3: 9 daily trades (High)
        let state = setup_state_max_trades().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("ETH").await;

        assert!(!result.passed, "8+ daily trades should block");
        assert_eq!(result.highest_failed_priority, Some(RulePriority::High));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "max_daily_trades"));

        // Scenario 4: Cooldown active (High)
        let state = setup_state_cooldown().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(!result.passed, "Active cooldown should block");
        assert_eq!(result.highest_failed_priority, Some(RulePriority::High));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "cooldown"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Medium blocks only if no Critical/High already set
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_medium_blocks_alone() {
        // Bear regime + low confluence → Medium blocks (no Critical/High)
        let state = setup_state_bear_regime_low_confluence().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(!result.passed, "Medium should block when no Higher rule overrides");
        assert_eq!(
            result.highest_failed_priority,
            Some(RulePriority::Medium),
            "Highest blocking priority should be Medium"
        );
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "regime_safety"));
    }

    #[tokio::test]
    async fn test_medium_does_not_override_critical() {
        // Critical (drawdown) + Medium (confluence) → Critical blocks
        let state = setup_state_drawdown().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::TrendingBear);
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(!result.passed, "Should block (Critical)");
        assert_eq!(
            result.highest_failed_priority,
            Some(RulePriority::Critical),
            "Critical should override Medium"
        );
        // Both rules should be in failed_rules
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "daily_drawdown"));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "regime_safety" || r.rule_name == "confluence_minimum"));
    }

    #[tokio::test]
    async fn test_medium_does_not_override_high() {
        // High (heat) + Medium (confluence) → High blocks
        let state = setup_state_high_heat().await;
        {
            *state.market_regime.write().await = None; // default confluence 0.5 < 0.65
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("SYM0").await;

        assert!(!result.passed, "Should block (High)");
        assert_eq!(
            result.highest_failed_priority,
            Some(RulePriority::High),
            "High should override Medium"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Low never blocks — warnings only
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_low_never_blocks() {
        // 3 positions on BTC → Low rule triggers but should NOT block
        let state = setup_state_max_positions_per_symbol().await;
        {
            // Also set high confluence so Medium rules pass
            let agg = tredo_core::skill_aggregator::AggregatedSignal {
                net_signal: 0.6,
                bullish_strength: 0.7,
                bearish_strength: 0.1,
                conviction: 0.85,
                consensus: Some(tredo_core::agent::SkillDirection::Bullish),
                participating_count: 5,
                bullish_count: 4,
                bearish_count: 1,
                neutral_count: 0,
            };
            *state.last_aggregated_signal.write().await = Some(agg);
            *state.market_regime.write().await = Some(MarketRegime::TrendingBull);
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(
            result.passed,
            "Low priority (max positions) should NOT block — only warn"
        );
        assert!(
            !result.failed_rules.is_empty(),
            "Should still record the Low-priority failure"
        );
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "max_positions_per_symbol"));
        // highest_failed_priority stays None because Low never sets it
        assert!(
            result.highest_failed_priority.is_none(),
            "Low should not set highest_blocking_priority"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Critical + Low together → Critical blocks, Low is warning
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_critical_overrides_low() {
        let state = setup_state_drawdown().await;
        {
            let mut portfolio = state.portfolio.write().await;
            // Also add 3 positions on BTC (Low rule)
            for i in 0..3 {
                portfolio.open_positions.push(OpenPosition {
                    symbol: "BTC".to_string(),
                    direction: TradeDirection::Long,
                    entry_price: 60000.0 + i as f64 * 1000.0,
                    current_price: 61000.0,
                    stop_loss: 58000.0,
                    take_profit: 65000.0,
                    quantity: 0.1,
                    unrealized_pnl: 0.0,
                    unrealized_pnl_pct: 0.0,
                    entry_time: Utc::now(),
                    risk_amount: 200.0,
                });
            }
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(!result.passed, "Critical should block even with Low warnings");
        assert_eq!(
            result.highest_failed_priority,
            Some(RulePriority::Critical),
            "Critical overrides Low"
        );
        // Both rules should be recorded
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "daily_drawdown"));
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "max_positions_per_symbol"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Multiple High rules → still blocks with High priority
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_multiple_high_rules() {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            portfolio.consecutive_losses = 5; // High
            portfolio.total_trades_today = 10; // High
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("ETH").await;

        assert!(!result.passed, "Multiple High rules should block");
        assert_eq!(
            result.highest_failed_priority,
            Some(RulePriority::High),
            "Highest should be High"
        );
        assert!(result.failed_rules.len() >= 2, "Should have at least 2 failed rules");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Confluence minimum varies by regime (Medium rule)
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_confluence_regime_adaptive() {
        // Ranging regime → min confluence = 0.70
        // Default confluence = 0.5 → should block
        let state = setup_state().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::Ranging);
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(
            !result.passed,
            "Confluence 0.5 < 0.70 (Ranging min) should block"
        );
        assert!(result
            .failed_rules
            .iter()
            .any(|r| r.rule_name == "confluence_minimum"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Crypto bypasses session timing (Critical)
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_crypto_bypasses_session_timing() {
        // BTC should pass even if session timing would fail for non-crypto
        let state = setup_state_high_confluence().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(
            result.passed,
            "BTC (crypto) should bypass session timing check"
        );
        assert!(
            !result
                .failed_rules
                .iter()
                .any(|r| r.rule_name == "session_timing"),
            "session_timing should not fail for crypto"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: HardRulesGateResult::passed() helper works
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_result_helper() {
        let result = HardRulesGateResult::passed();
        assert!(result.passed);
        assert!(result.failed_rules.is_empty());
        assert!(result.highest_failed_priority.is_none());
        assert_eq!(result.total_rules_checked, 0);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: RulePriority ordering (Critical > High > Medium > Low)
    // ═══════════════════════════════════════════════════════════════════════
    #[test]
    fn test_priority_ordering() {
        assert!(RulePriority::Critical > RulePriority::High);
        assert!(RulePriority::High > RulePriority::Medium);
        assert!(RulePriority::Medium > RulePriority::Low);
        assert!(RulePriority::Critical > RulePriority::Low);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: All 12 rules checked (total_rules_checked = 12)
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_all_12_rules_checked() {
        let state = setup_state_high_confluence().await;
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert_eq!(
            result.total_rules_checked, 12,
            "Should check all 12 rules regardless of pass/fail"
        );
    }
}
