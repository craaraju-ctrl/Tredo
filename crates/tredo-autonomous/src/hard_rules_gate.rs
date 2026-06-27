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
use crate::types::{HardRulesGateResult, OhlcvSnapshot, RuleCheck, RulePriority};
use chrono::Utc;

pub struct HardRulesGate {
    state: SharedState,
}

impl HardRulesGate {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Run ALL hard rules in priority order using the default SharedState.
    /// Delegates to [`evaluate_with_ohlcv`] with a live snapshot from SharedState.
    pub async fn evaluate(&self, symbol: &str) -> HardRulesGateResult {
        let snapshot = OhlcvSnapshot::capture(symbol, &self.state).await;
        self.evaluate_with_ohlcv(symbol, &snapshot).await
    }

    /// Run ALL hard rules using an explicit OHLCV snapshot so all 3 verification
    /// layers (HardRulesGate, LLM, Kronos) see the identical market data.
    ///
    /// Priority-based blocking logic:
    /// - Critical/High failures: ALWAYS block (no override possible)
    /// - Medium failures: block ONLY if no Critical/High rule has already been checked
    ///   (i.e., Medium rules are soft-blocks that can be overridden by a Higher rule passing)
    /// - Low failures: WARNINGS ONLY — logged but never block
    ///
    /// This prevents a Low-priority position-size preference from blocking a
    /// Critical drawdown halt, while still enforcing Medium-priority regime checks.
    pub async fn evaluate_with_ohlcv(
        &self,
        symbol: &str,
        snapshot: &OhlcvSnapshot,
    ) -> HardRulesGateResult {
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
            let is_crypto = tredo_core::is_crypto_symbol(symbol);
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
                portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.risk_amount)
                    .sum::<f64>()
                    / portfolio.total_equity
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
                if highest_blocking_priority.is_none()
                    || highest_blocking_priority == Some(RulePriority::Medium)
                {
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
                    reason: format!(
                        "{} consecutive losses — circuit breaker triggered",
                        portfolio.consecutive_losses
                    ),
                });
                if highest_blocking_priority.is_none()
                    || highest_blocking_priority == Some(RulePriority::Medium)
                {
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
                    reason: format!(
                        "{} trades today exceeds 8-trade daily limit",
                        portfolio.total_trades_today
                    ),
                });
                if highest_blocking_priority.is_none()
                    || highest_blocking_priority == Some(RulePriority::Medium)
                {
                    highest_blocking_priority = Some(RulePriority::High);
                }
            }
        }

        // 8. Cooldown check — per-symbol (trading ETH must not block on a prior BTC trade).
        // Paper mode uses a shorter cooldown for observation/testing.
        total_checked += 1;
        {
            let rules = self.state.rules.read().await;
            let mut cooldown = rules.cooldown_secs;
            if self.state.config.paper_mode {
                cooldown = cooldown.min(60);
            }
            drop(rules);

            let portfolio = self.state.portfolio.read().await;
            let last_trade_time =
                portfolio
                    .last_trade_by_symbol
                    .get(symbol)
                    .copied()
                    .or_else(|| {
                        if portfolio.last_trade_symbol.as_deref() == Some(symbol) {
                            portfolio.last_trade_time
                        } else {
                            None
                        }
                    });
            if let Some(last_trade_time) = last_trade_time {
                let elapsed = Utc::now() - last_trade_time;
                if elapsed.num_seconds() < cooldown as i64 {
                    failed_rules.push(RuleCheck {
                        passed: false,
                        rule_name: "cooldown".to_string(),
                        priority: RulePriority::High,
                        reason: format!(
                            "{}s since last {} trade (min {}s)",
                            elapsed.num_seconds(),
                            symbol,
                            cooldown
                        ),
                    });
                    if highest_blocking_priority.is_none()
                        || highest_blocking_priority == Some(RulePriority::Medium)
                    {
                        highest_blocking_priority = Some(RulePriority::High);
                    }
                }
            }
        }

        // ── AUTO-REGIME INFERENCE (Bootstrapping fix) ────────────────────────
        // If market_regime is None (first run / no skills yet), compute a
        // preliminary regime from the OHLCV snapshot so the pipeline can proceed.
        // Uses the same snapshot data as LLM and Kronos — all 3 layers see identical data.
        {
            let regime = *self.state.market_regime.read().await;
            if regime.is_none() {
                if snapshot.len() >= 50 {
                    let prices: Vec<f64> = snapshot.bars().iter().map(|b| b.close).collect();
                    let highs: Vec<f64> = snapshot.bars().iter().map(|b| b.high).collect();
                    let lows: Vec<f64> = snapshot.bars().iter().map(|b| b.low).collect();
                    let inferred = crate::helpers::estimate_market_regime(&prices, &highs, &lows);
                    *self.state.market_regime.write().await = Some(inferred);
                    println!(
                        "[HardRulesGate] 🧭 Auto-inferred regime for {}: {:?} (from {} snapshot bars)",
                        symbol,
                        inferred,
                        snapshot.len()
                    );
                } else {
                    println!(
                        "[HardRulesGate] ⏳ Bootstrapping {} — only {} snapshot bars (need 50+), using relaxed thresholds",
                        symbol,
                        snapshot.len()
                    );
                }
            }
        }

        // ── MEDIUM PRIORITY (Regime, confluence) ────────────────────────────

        // 9. Regime safety: no BUY in bear regime with low confluence
        total_checked += 1;
        {
            let regime = *self.state.market_regime.read().await;
            if regime == Some(crate::types::MarketRegime::TrendingBear) {
                let base_confluence = {
                    let rules = self.state.rules.read().await;
                    rules.min_confluence_score
                };
                // Bear regime threshold: same as configured min_confluence_score (no penalty offset)
                // With base=0.30: threshold = 0.30. With base=0.65: threshold = 0.65
                let bear_threshold = base_confluence;
                let confluence =
                    crate::helpers::resolve_symbol_confluence(&self.state, symbol).await;
                if confluence < bear_threshold {
                    failed_rules.push(RuleCheck {
                        passed: false,
                        rule_name: "regime_safety".to_string(),
                        priority: RulePriority::Medium,
                        reason: format!(
                            "Bear regime with confluence {:.1}% < {:.0}% minimum",
                            confluence * 100.0,
                            bear_threshold * 100.0
                        ),
                    });
                    // Medium only blocks if no Critical/High already set
                    if highest_blocking_priority.is_none() {
                        highest_blocking_priority = Some(RulePriority::Medium);
                    }
                }
            }
        }

        // 10. Confluence minimum (regime-adaptive)
        // Uses snapshot bar count so all layers see the same data volume.
        total_checked += 1;
        {
            let confluence = crate::helpers::resolve_symbol_confluence(&self.state, symbol).await;
            let agg_is_none = self.state.last_aggregated_signal.read().await.is_none();
            let regime = *self.state.market_regime.read().await;
            let bars_count = snapshot.len();
            // Bootstrapping: allow through if we have little data OR if the aggregated
            // signal has never been populated (skills haven't run yet). This prevents a
            // deadlock where the gate blocks before skills can seed the confluence data.
            let is_bootstrapping = bars_count < 100 || agg_is_none;
            // During bootstrapping, use a relaxed threshold regardless of regime.
            // The auto-regime inference may have set TrendingBear (0.80) or Volatile
            // (0.75) before skills have run — the confluence check would fail with
            // the higher regime-specific threshold, deadlocking the pipeline.
            // By applying the bootstrapping override to ALL regimes, we ensure the
            // first pipeline runs always pass through so skills can seed data.
            let base_confluence = {
                let rules = self.state.rules.read().await;
                rules.min_confluence_score
            };
            // Regime-adaptive thresholds anchored to configurable base.
            // Each regime adjusts by a fixed offset from the configured base.
            // base=0.30: bull=0.30, ranging=0.30, volatile=0.40, bear=0.30, lowliq=0.50
            // base=0.65: bull=0.65, ranging=0.65, volatile=0.75, bear=0.65, lowliq=0.85 (old defaults)
            let mut min_confluence = match &regime {
                Some(crate::types::MarketRegime::TrendingBull) => {
                    (base_confluence - 0.15).max(0.30)
                }
                Some(crate::types::MarketRegime::TrendingBear) => base_confluence,
                Some(crate::types::MarketRegime::Ranging) => base_confluence,
                Some(crate::types::MarketRegime::Volatile) => base_confluence + 0.10,
                Some(crate::types::MarketRegime::LowLiquidity) => {
                    (base_confluence + 0.20).min(0.90)
                }
                None => base_confluence,
            };
            if is_bootstrapping {
                min_confluence = 0.35;
            }
            if confluence < min_confluence {
                let reason = if is_bootstrapping {
                    format!(
                        "Confluence {:.1}% below bootstrap minimum {:.1}% ({} bars) — allowing pass to seed skills",
                        confluence * 100.0,
                        min_confluence * 100.0,
                        bars_count
                    )
                } else {
                    format!(
                        "Confluence {:.1}% below regime minimum {:.1}%",
                        confluence * 100.0,
                        min_confluence * 100.0
                    )
                };

                // During bootstrapping (low bars for THIS symbol), allow through
                // so the skills can seed data for this specific symbol.
                // Note: regime may be set from another symbol's data, so we check
                // is_bootstrapping directly rather than regime.is_none().
                if is_bootstrapping {
                    println!(
                        "[HardRulesGate] ⏩ {} (bootstrap override — not blocking, {} bars)",
                        reason, bars_count
                    );
                    // Don't push to failed_rules — allow through
                } else {
                    failed_rules.push(RuleCheck {
                        passed: false,
                        rule_name: "confluence_minimum".to_string(),
                        priority: RulePriority::Medium,
                        reason,
                    });
                    if highest_blocking_priority.is_none() {
                        highest_blocking_priority = Some(RulePriority::Medium);
                    }
                }
            }
        }

        // ── LOW PRIORITY (Position limits — WARNINGS ONLY) ──────────────────

        // 11. Open position check (max 3 per symbol)
        total_checked += 1;
        {
            let portfolio = self.state.portfolio.read().await;
            let sym_positions = portfolio
                .open_positions
                .iter()
                .filter(|p| p.symbol == symbol)
                .count();
            if sym_positions >= 3 {
                failed_rules.push(RuleCheck {
                    passed: false,
                    rule_name: "max_positions_per_symbol".to_string(),
                    priority: RulePriority::Low,
                    reason: format!(
                        "{} positions on {} — max 3 per symbol",
                        sym_positions, symbol
                    ),
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
                    reason: format!(
                        "{} open positions — max 10 total",
                        portfolio.open_positions.len()
                    ),
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
                    failed_rules.len(),
                    symbol
                );
                for rule in &failed_rules {
                    println!("  - [WARNING] {}: {}", rule.rule_name, rule.reason);
                }
            } else {
                println!(
                    "[HardRulesGate] ✅ All {} rules passed for {}",
                    total_checked, symbol
                );
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
                    println!(
                        "  - [BLOCK] [{:?}] {}: {}",
                        rule.priority, rule.rule_name, rule.reason
                    );
                } else {
                    println!(
                        "  - [WARN] [{:?}] {}: {}",
                        rule.priority, rule.rule_name, rule.reason
                    );
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
    /// Calendar events are cleared so the red_folder Critical rule doesn't
    /// interfere with priority-level testing.
    async fn setup_state() -> SharedState {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let redb_path = format!("file:test_hrg_{}.redb?mode=memory", id);
        let memory = MemoryStore::new(&redb_path).expect("MemoryStore creation");
        let config = Config {
            kronos_service_url: "http://127.0.0.1:19999".to_string(),
            ..Config::default()
        };
        let rules = DisciplineRules::default();
        let state = SharedState::new(memory, rules, config, ":memory:").expect("SharedState init");
        // Clear calendar events so red_folder Critical rule doesn't fire in tests
        *state.calendar_events.write().await = Vec::new();
        state
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
    /// Default cooldown is 1800s, so 10s ago ensures it blocks.
    async fn setup_state_cooldown() -> SharedState {
        let state = setup_state().await;
        {
            let mut portfolio = state.portfolio.write().await;
            let ts = Utc::now() - Duration::seconds(10);
            portfolio.last_trade_time = Some(ts);
            portfolio.last_trade_symbol = Some("BTC".to_string());
            portfolio.last_trade_by_symbol.insert("BTC".to_string(), ts);
        }
        state
    }

    /// Create a SharedState with bear regime + low confluence (Medium rule).
    /// Seeds conviction=0.25 so the bear regime safety rule (threshold=0.30) triggers.
    async fn setup_state_bear_regime_low_confluence() -> SharedState {
        let state = setup_state().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::TrendingBear);
            let agg = tredo_core::skill_aggregator::AggregatedSignal {
                net_signal: -0.3,
                bullish_strength: 0.2,
                bearish_strength: 0.6,
                conviction: 0.25, // below bear threshold 0.30
                consensus: Some(tredo_core::agent::SkillDirection::Bearish),
                participating_count: 3,
                bullish_count: 1,
                bearish_count: 2,
                neutral_count: 0,
            };
            *state.last_aggregated_signal.write().await = Some(agg);
        }
        state
    }

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
        let result = gate.evaluate("BTC").await;

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

        assert!(
            !result.passed,
            "Medium should block when no Higher rule overrides"
        );
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
        // Seed low conviction so bear regime safety rule triggers
        let state = setup_state_drawdown().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::TrendingBear);
            let agg = tredo_core::skill_aggregator::AggregatedSignal {
                net_signal: -0.3,
                bullish_strength: 0.2,
                bearish_strength: 0.6,
                conviction: 0.25, // below bear threshold 0.30
                consensus: Some(tredo_core::agent::SkillDirection::Bearish),
                participating_count: 3,
                bullish_count: 1,
                bearish_count: 2,
                neutral_count: 0,
            };
            *state.last_aggregated_signal.write().await = Some(agg);
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
        // Set regime to TrendingBear for high threshold, and seed low conviction
        let state = setup_state_high_heat().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::TrendingBear);
            let agg = tredo_core::skill_aggregator::AggregatedSignal {
                net_signal: 0.1,
                bullish_strength: 0.2,
                bearish_strength: 0.3,
                conviction: 0.25, // below base=0.30 bear threshold
                consensus: Some(tredo_core::agent::SkillDirection::Bearish),
                participating_count: 3,
                bullish_count: 1,
                bearish_count: 2,
                neutral_count: 0,
            };
            *state.last_aggregated_signal.write().await = Some(agg);
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

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

        assert!(
            !result.passed,
            "Critical should block even with Low warnings"
        );
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
        assert!(
            result.failed_rules.len() >= 2,
            "Should have at least 2 failed rules"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // TEST: Confluence minimum varies by regime (Medium rule)
    // ═══════════════════════════════════════════════════════════════════════
    #[tokio::test]
    async fn test_confluence_regime_adaptive() {
        // Ranging regime → min confluence = base = 0.30
        // Seed low confluence (0.25) → should block
        // Must seed 101+ bars to bypass the bootstrap override (<100 bars skips failure)
        let state = setup_state().await;
        {
            *state.market_regime.write().await = Some(MarketRegime::Ranging);
            let agg = tredo_core::skill_aggregator::AggregatedSignal {
                net_signal: 0.1,
                bullish_strength: 0.2,
                bearish_strength: 0.3,
                conviction: 0.25, // below 0.30 threshold
                consensus: Some(tredo_core::agent::SkillDirection::Neutral),
                participating_count: 3,
                bullish_count: 1,
                bearish_count: 2,
                neutral_count: 0,
            };
            *state.last_aggregated_signal.write().await = Some(agg);
            // Seed 101 bars so is_bootstrapping = false and actual threshold applies
            let bar = tredo_core::OhlcvBar {
                timestamp: "2026-01-01T00:00:00+00:00".to_string(),
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 1000.0,
            };
            state
                .ohlcv_history
                .write()
                .await
                .insert("BTC".to_string(), vec![bar; 101]);
        }
        let gate = HardRulesGate::new(state);
        let result = gate.evaluate("BTC").await;

        assert!(
            !result.passed,
            "Confluence 0.25 < 0.30 (Ranging min) should block"
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
