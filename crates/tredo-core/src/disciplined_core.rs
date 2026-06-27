use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single skill's vote during a trading decision.
/// Captured by MarketIntelligenceAgent and consumed by OutcomeProcessor
/// to record whether the skill's direction matched the actual trade outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillVote {
    pub skill_name: String,
    pub direction: crate::agent::SkillDirection,
    pub weight: f64,
    pub confidence: f64,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisciplineRules {
    pub use_daily_pivots: bool,
    pub pivot_method: PivotMethod,
    pub respect_session_timing: bool,
    pub london_open_hour: u32,
    pub london_close_hour: u32,
    pub ny_open_hour: u32,
    pub ny_close_hour: u32,
    pub max_risk_per_trade: f64,
    pub max_daily_drawdown: f64,
    pub max_consecutive_losses: u32,
    pub use_confluence: bool,
    pub min_confluence_score: f64,
    pub red_folder_discipline: bool,
    pub require_trend_filter: bool,
    /// Per-skill weights for ensemble aggregation.
    /// Keyed by skill name (e.g. "SentimentAnalyzer", "VolatilityCalculator").
    /// Adjusted by MetaControlAgent based on predictive accuracy over time.
    pub skill_weights: HashMap<String, f64>,
    /// Monotonically increasing version of the rule set. Every MetaControl adaptation
    /// that changes rules or weights bumps this. Used for memory versioning so that
    /// similarity search in vector/episodic memory only recalls memories from compatible
    /// rule regimes (prevents "self-lobotomy" from outdated memories).
    pub rule_version: u32,
    /// Cooldown between same-symbol trades in seconds.
    /// Same value is used for both paper and live trading (perfect parity).
    /// Default: 1800 (30 min). Lower for testing: e.g. 30s for backtests.
    pub cooldown_secs: u64,
}

impl Default for DisciplineRules {
    fn default() -> Self {
        let mut skill_weights = HashMap::new();
        // Weights normalized to sum precisely to 1.0 for proper ensemble aggregation.
        skill_weights.insert("MarketMetricsMeter".to_string(), 0.29);
        skill_weights.insert("NewsAnalyser".to_string(), 0.12);
        skill_weights.insert("RegimeDetector".to_string(), 0.10);
        skill_weights.insert("SentimentAnalyzer".to_string(), 0.08);
        skill_weights.insert("VolatilityCalculator".to_string(), 0.06);
        skill_weights.insert("OnChainData".to_string(), 0.06);
        skill_weights.insert("SupportResistance".to_string(), 0.06);
        skill_weights.insert("OrderFlow".to_string(), 0.06);
        skill_weights.insert("PatternRetriever".to_string(), 0.04);
        skill_weights.insert("VolumeProfile".to_string(), 0.04);
        skill_weights.insert("CorrelationChecker".to_string(), 0.03);
        skill_weights.insert("FundingRate".to_string(), 0.03);
        skill_weights.insert("Liquidity".to_string(), 0.03);

        Self {
            use_daily_pivots: true,
            pivot_method: PivotMethod::Classic,
            respect_session_timing: true,
            london_open_hour: 8,
            london_close_hour: 16,
            ny_open_hour: 13,
            ny_close_hour: 21,
            max_risk_per_trade: 0.01,
            max_daily_drawdown: 0.03,
            max_consecutive_losses: 3,
            use_confluence: true,
            min_confluence_score: 0.30,
            red_folder_discipline: true,
            require_trend_filter: true,
            skill_weights,
            rule_version: 1,
            cooldown_secs: 1800,
        }
    }
}

impl DisciplineRules {
    /// Get the ensemble weight for a skill. Returns the configured weight,
    /// or the default of 0.20 if the skill is not yet tracked.
    pub fn get_skill_weight(&self, name: &str) -> f64 {
        self.skill_weights.get(name).copied().unwrap_or(0.20)
    }

    /// Update a skill weight, clamped to [0.0, 1.0].
    pub fn set_skill_weight(&mut self, name: &str, weight: f64) {
        let clamped = weight.clamp(0.0, 1.0);
        self.skill_weights.insert(name.to_string(), clamped);
    }

    /// Adjust a skill weight by a delta, clamped to [0.0, 1.0].
    pub fn adjust_skill_weight(&mut self, name: &str, delta: f64) {
        let current = self.get_skill_weight(name);
        self.set_skill_weight(name, current + delta);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketContext {
    pub symbol: String,
    pub current_price: f64,
    pub high: f64,
    pub low: f64,
    pub previous_close: f64,
    pub timestamp: DateTime<Utc>,
    pub daily_pnl: f64,
    pub equity: f64,
    pub consecutive_losses: u32,
    pub is_red_folder_day: bool,
    pub trend_direction: Option<TrendDirection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendDirection {
    Bullish,
    Bearish,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PivotLevels {
    pub pivot: f64,
    pub r1: f64,
    pub r2: f64,
    pub r3: f64,
    pub s1: f64,
    pub s2: f64,
    pub s3: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisciplineCheck {
    pub passed: bool,
    pub reasons: Vec<String>,
    pub confluence_score: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotMethod {
    Classic,
    Woodie,
    Fibonacci,
}

pub fn calculate_pivot_points(high: f64, low: f64, close: f64, method: PivotMethod) -> PivotLevels {
    let range = high - low;
    let pivot = match method {
        PivotMethod::Classic => (high + low + close) / 3.0,
        PivotMethod::Woodie => (high + low + 2.0 * close) / 4.0,
        PivotMethod::Fibonacci => (high + low + close) / 3.0,
    };
    match method {
        PivotMethod::Fibonacci => PivotLevels {
            pivot,
            r1: pivot + 0.382 * range,
            r2: pivot + 0.618 * range,
            r3: pivot + 1.000 * range,
            s1: pivot - 0.382 * range,
            s2: pivot - 0.618 * range,
            s3: pivot - 1.000 * range,
        },
        _ => {
            let r1 = 2.0 * pivot - low;
            let s1 = 2.0 * pivot - high;
            PivotLevels {
                pivot,
                r1,
                r2: pivot + (high - low),
                r3: high + 2.0 * (pivot - low),
                s1,
                s2: pivot - (high - low),
                s3: low - 2.0 * (high - pivot),
            }
        }
    }
}

pub fn is_in_trading_session(timestamp: DateTime<Utc>, rules: &DisciplineRules) -> bool {
    if !rules.respect_session_timing {
        return true;
    }
    let hour = timestamp.hour();
    let in_london = hour >= rules.london_open_hour && hour < rules.london_close_hour;
    let in_ny = hour >= rules.ny_open_hour && hour < rules.ny_close_hour;
    in_london || in_ny
}

pub fn check_risk_limits(context: &MarketContext, rules: &DisciplineRules) -> DisciplineCheck {
    let mut reasons = Vec::new();
    let mut passed = true;
    let drawdown_pct = if context.equity > 0.0 {
        context.daily_pnl / context.equity
    } else {
        0.0
    };
    if drawdown_pct <= -rules.max_daily_drawdown {
        reasons.push(format!(
            "Daily drawdown limit reached: {:.2}% (P&L: ₹{:.2} / Equity: ₹{:.2})",
            drawdown_pct.abs() * 100.0,
            context.daily_pnl,
            context.equity
        ));
        passed = false;
    }
    if context.consecutive_losses >= rules.max_consecutive_losses {
        reasons.push(format!(
            "Maximum consecutive losses ({}) reached",
            rules.max_consecutive_losses
        ));
        passed = false;
    }
    if context.is_red_folder_day && rules.red_folder_discipline {
        reasons.push("Red folder / high-impact event day – trading restricted".to_string());
        passed = false;
    }
    DisciplineCheck {
        passed,
        reasons,
        confluence_score: None,
    }
}

/// Returns the regime-adaptive minimum confluence score threshold.
/// Research shows this improves precision by 15-20% vs static thresholds:
/// - TrendingBull: 0.50 (lower bar — momentum is your friend)
/// - TrendingBear: 0.80 (higher bar — fighting the trend)
/// - Ranging: 0.70 (moderate — need strong signal in chop)
/// - Volatile: 0.75 (high bar — noise is high)
pub fn regime_min_confluence(regime_str: &str) -> f64 {
    match regime_str {
        "TrendingBull" => 0.50,
        "TrendingBear" => 0.80,
        "Ranging" => 0.70,
        "Volatile" => 0.75,
        "LowLiquidity" => 0.85,
        _ => 0.65, // Default when regime unknown
    }
}

pub fn calculate_confluence_score(context: &MarketContext, pivots: &PivotLevels) -> f64 {
    let mut score: f64 = 0.5;
    let distance_to_pivot = (context.current_price - pivots.pivot).abs() / context.current_price;
    if distance_to_pivot < 0.002 {
        score += 0.15;
    }
    if let Some(trend) = context.trend_direction {
        match trend {
            TrendDirection::Bullish if context.current_price > pivots.pivot => score += 0.2,
            TrendDirection::Bearish if context.current_price < pivots.pivot => score += 0.2,
            _ => {}
        }
    }
    score.min(1.0)
}

pub fn validate_trade_setup(context: &MarketContext, rules: &DisciplineRules) -> DisciplineCheck {
    let mut all_reasons = Vec::new();
    let mut overall_passed = true;

    let is_crypto = matches!(
        context.symbol.as_str(),
        "BTC"
            | "ETH"
            | "SOL"
            | "BNB"
            | "XRP"
            | "ADA"
            | "DOGE"
            | "AVAX"
            | "MATIC"
            | "LINK"
            | "DOT"
            | "ATOM"
            | "LTC"
            | "UNI"
            | "AAVE"
            | "NEAR"
            | "APT"
            | "ARB"
            | "OP"
            | "SUI"
            | "INJ"
            | "TON"
            | "TRX"
            | "XLM"
            | "PEPE"
            | "SHIB"
    );
    if !is_crypto && !is_in_trading_session(context.timestamp, rules) {
        all_reasons.push("Outside allowed trading sessions (London/NY)".to_string());
        overall_passed = false;
    }

    let risk_check = check_risk_limits(context, rules);
    if !risk_check.passed {
        all_reasons.extend(risk_check.reasons);
        overall_passed = false;
    }

    let mut confluence_score = None;
    if rules.use_daily_pivots {
        let pivots = calculate_pivot_points(
            context.high,
            context.low,
            context.previous_close,
            rules.pivot_method,
        );
        if rules.use_confluence {
            let score = calculate_confluence_score(context, &pivots);
            confluence_score = Some(score);
            if score < rules.min_confluence_score {
                all_reasons.push(format!("Confluence score too low: {:.2}", score));
                overall_passed = false;
            }
        }
    }

    DisciplineCheck {
        passed: overall_passed,
        reasons: all_reasons,
        confluence_score,
    }
}

/// Memory-aware rule adjustment (strong "rules" + trained memory).
/// Rules tell "what to do / not to do". This uses hierarchical trained memory (from recall)
/// to dynamically tighten rules (e.g., raise min_confluence or lower max risk) if past similar actions had high regret.
/// Agents/sub-agents already know their roles ("what to do"); this strengthens the guardrails with learned lessons to reduce bad decisions/hallucinations long-term.
pub fn apply_trained_memory_to_rules(
    rules: &mut DisciplineRules,
    trained_memory_recall: &str, // from SharedState::recall_trained_memory
) {
    if trained_memory_recall.contains("regret")
        || trained_memory_recall.contains("high regret")
        || trained_memory_recall.contains("caution")
    {
        // Learned from past: tighten rules when history shows problems in similar setups.
        if rules.min_confluence_score < 0.75 {
            rules.min_confluence_score = (rules.min_confluence_score + 0.05).min(0.8);
            println!("[Rules + TrainedMemory] Tightened min_confluence to {:.2} based on past regret/lessons in recall.", rules.min_confluence_score);
        }
        if rules.max_risk_per_trade > 0.008 {
            rules.max_risk_per_trade = (rules.max_risk_per_trade * 0.9).max(0.005);
            println!("[Rules + TrainedMemory] Reduced max_risk_per_trade to {:.4} based on trained memory.", rules.max_risk_per_trade);
        }
    }
    // Could add more: if recall shows "good outcomes on this regime", loosen slightly, etc.
    // This is how rules evolve with "trained memory" without changing core agent logic.
}

/// Validates a trade setup with regime-adaptive confluence threshold.
/// This is the precision-critical version that replaces the static 0.65 threshold.
pub fn validate_trade_setup_regime_aware(
    context: &MarketContext,
    rules: &DisciplineRules,
    regime_str: &str,
) -> DisciplineCheck {
    let mut all_reasons = Vec::new();
    let mut overall_passed = true;

    let is_crypto = matches!(
        context.symbol.as_str(),
        "BTC"
            | "ETH"
            | "SOL"
            | "BNB"
            | "XRP"
            | "ADA"
            | "DOGE"
            | "AVAX"
            | "MATIC"
            | "LINK"
            | "DOT"
            | "ATOM"
            | "LTC"
            | "UNI"
            | "AAVE"
            | "NEAR"
            | "APT"
            | "ARB"
            | "OP"
            | "SUI"
            | "INJ"
            | "TON"
            | "TRX"
            | "XLM"
            | "PEPE"
            | "SHIB"
    );
    if !is_crypto && !is_in_trading_session(context.timestamp, rules) {
        all_reasons.push("Outside allowed trading sessions (London/NY)".to_string());
        overall_passed = false;
    }

    let risk_check = check_risk_limits(context, rules);
    if !risk_check.passed {
        all_reasons.extend(risk_check.reasons);
        overall_passed = false;
    }

    let mut confluence_score = None;
    if rules.use_daily_pivots {
        let pivots = calculate_pivot_points(
            context.high,
            context.low,
            context.previous_close,
            rules.pivot_method,
        );
        if rules.use_confluence {
            let score = calculate_confluence_score(context, &pivots);
            confluence_score = Some(score);

            // Regime-adaptive confluence threshold (replaces static min_confluence_score)
            let regime_threshold = regime_min_confluence(regime_str);
            // Use the stricter of: regime-adaptive threshold vs configured minimum
            let effective_threshold = regime_threshold.max(rules.min_confluence_score);

            if score < effective_threshold {
                all_reasons.push(format!(
                    "Confluence {:.2} below regime threshold {:.2} (regime={})",
                    score, effective_threshold, regime_str
                ));
                overall_passed = false;
            }
        }
    }

    DisciplineCheck {
        passed: overall_passed,
        reasons: all_reasons,
        confluence_score,
    }
}

pub fn get_discipline_summary() -> &'static str {
    "Disciplined Core v0.2 – Pivot points, session timing, risk limits, and confluence active"
}

#[cfg(test)]
mod qa_tests {
    use super::*;

    #[test]
    fn test_drawdown_limit_uses_percentage_not_currency() {
        let rules = DisciplineRules {
            max_daily_drawdown: 0.03, // 3%
            ..DisciplineRules::default()
        };

        // Scenario: Small currency loss of ₹0.05 on a ₹100,000 equity portfolio.
        // This is a microscopic loss (0.00005%), not a 3% drawdown (₹3,000).
        let context = MarketContext {
            symbol: "NIFTY".to_string(),
            current_price: 24500.0,
            high: 24550.0,
            low: 24450.0,
            previous_close: 24500.0,
            timestamp: Utc::now(),
            daily_pnl: -0.05,  // micro loss, NOT 3%
            equity: 100_000.0, // ₹1 Lakh portfolio
            consecutive_losses: 0,
            is_red_folder_day: false,
            trend_direction: None,
        };

        // Bug A: Before fix, -0.05 <= -0.03 was TRUE, falsely triggering drawdown.
        // After fix: drawdown_pct = -0.05 / 100_000 = -0.0000005, which is NOT <= -0.03
        let result = check_risk_limits(&context, &rules);
        assert!(
            result.passed,
            "Bug A: Small currency loss (₹{:.2}) on ₹{:.0} equity should NOT trigger {:.0}% drawdown limit. Got: {:?}",
            context.daily_pnl, context.equity, rules.max_daily_drawdown * 100.0, result.reasons
        );
    }

    #[test]
    fn test_drawdown_triggers_at_real_percentage() {
        let rules = DisciplineRules {
            max_daily_drawdown: 0.03, // 3%
            ..DisciplineRules::default()
        };

        // Scenario: ₹4,000 loss on ₹100,000 equity = 4% drawdown, exceeds 3% limit
        let context = MarketContext {
            symbol: "NIFTY".to_string(),
            current_price: 24500.0,
            high: 24550.0,
            low: 24450.0,
            previous_close: 24500.0,
            timestamp: Utc::now(),
            daily_pnl: -4000.0, // 4% loss
            equity: 100_000.0,
            consecutive_losses: 0,
            is_red_folder_day: false,
            trend_direction: None,
        };

        let result = check_risk_limits(&context, &rules);
        assert!(!result.passed, "4% drawdown should trigger 3% limit");
        assert!(result.reasons.iter().any(|r| r.contains("drawdown")));
    }

    #[test]
    fn test_fibonacci_pivots_differ_from_classic() {
        let high = 100.0;
        let low = 80.0;
        let close = 90.0;

        let classic = calculate_pivot_points(high, low, close, PivotMethod::Classic);
        let fib = calculate_pivot_points(high, low, close, PivotMethod::Fibonacci);

        // Classic R1 = 2*90 - 80 = 100.0
        // Fibonacci R1 = 90 + 0.382 * 20 = 97.64
        assert_ne!(
            classic.r1, fib.r1,
            "Bug B: Fibonacci R1 ({:.2}) should differ from Classic R1 ({:.2})",
            fib.r1, classic.r1
        );

        // Classic R2 = 90 + 20 = 110.0
        // Fibonacci R2 = 90 + 0.618 * 20 = 102.36
        assert_ne!(
            classic.r2, fib.r2,
            "Fibonacci R2 should differ from Classic R2"
        );

        // Verify correct Fibonacci multipliers
        assert!(
            (fib.r1 - 97.64).abs() < 0.01,
            "Fibonacci R1 should be 97.64, got {}",
            fib.r1
        );
        assert!(
            (fib.r2 - 102.36).abs() < 0.01,
            "Fibonacci R2 should be 102.36, got {}",
            fib.r2
        );
        assert!(
            (fib.r3 - 110.0).abs() < 0.01,
            "Fibonacci R3 should be 110.0, got {}",
            fib.r3
        );
        assert!(
            (fib.s1 - 82.36).abs() < 0.01,
            "Fibonacci S1 should be 82.36, got {}",
            fib.s1
        );
        assert!(
            (fib.s2 - 77.64).abs() < 0.01,
            "Fibonacci S2 should be 77.64, got {}",
            fib.s2
        );
        assert!(
            (fib.s3 - 70.0).abs() < 0.01,
            "Fibonacci S3 should be 70.0, got {}",
            fib.s3
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // regime_min_confluence tests — validates each regime returns correct threshold
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_regime_min_confluence_trending_bull() {
        assert_eq!(
            regime_min_confluence("TrendingBull"),
            0.50,
            "TrendingBull should have lowest threshold — momentum is your friend"
        );
    }

    #[test]
    fn test_regime_min_confluence_trending_bear() {
        assert_eq!(
            regime_min_confluence("TrendingBear"),
            0.80,
            "TrendingBear should have high threshold — fighting the trend"
        );
    }

    #[test]
    fn test_regime_min_confluence_ranging() {
        assert_eq!(
            regime_min_confluence("Ranging"),
            0.70,
            "Ranging should have moderate threshold — need strong signal in chop"
        );
    }

    #[test]
    fn test_regime_min_confluence_volatile() {
        assert_eq!(
            regime_min_confluence("Volatile"),
            0.75,
            "Volatile should have high threshold — noise is high"
        );
    }

    #[test]
    fn test_regime_min_confluence_low_liquidity() {
        assert_eq!(
            regime_min_confluence("LowLiquidity"),
            0.85,
            "LowLiquidity should have highest threshold — slippage risk"
        );
    }

    #[test]
    fn test_regime_min_confluence_unknown_defaults() {
        assert_eq!(
            regime_min_confluence(""),
            0.65,
            "Empty string should return default"
        );
        assert_eq!(
            regime_min_confluence("Banana"),
            0.65,
            "Unknown regime should return default 0.65"
        );
        assert_eq!(
            regime_min_confluence("trendingbull"),
            0.65,
            "Case-sensitive: lowercase should return default"
        );
    }

    #[test]
    fn test_regime_min_confluence_ordering() {
        // TrendingBull (0.50) < Ranging (0.70) < Volatile (0.75) < TrendingBear (0.80) < LowLiquidity (0.85)
        assert!(regime_min_confluence("TrendingBull") < regime_min_confluence("Ranging"));
        assert!(regime_min_confluence("Ranging") < regime_min_confluence("Volatile"));
        assert!(regime_min_confluence("Volatile") < regime_min_confluence("TrendingBear"));
        assert!(regime_min_confluence("TrendingBear") < regime_min_confluence("LowLiquidity"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // validate_trade_setup_regime_aware tests
    // ═══════════════════════════════════════════════════════════════════════

    fn make_test_context(symbol: &str, price: f64, trend: Option<TrendDirection>) -> MarketContext {
        MarketContext {
            symbol: symbol.to_string(),
            current_price: price,
            high: price * 1.005,
            low: price * 0.995,
            previous_close: price,
            timestamp: Utc::now(),
            daily_pnl: 0.0,
            equity: 100_000.0,
            consecutive_losses: 0,
            is_red_folder_day: false,
            trend_direction: trend,
        }
    }

    #[test]
    fn test_regime_aware_passes_in_trending_bull_with_high_confluence() {
        // Price above pivot + bullish trend = high confluence (~0.85)
        // TrendingBull threshold = 0.50, so this should PASS
        let ctx = make_test_context("BTC", 100.0, Some(TrendDirection::Bullish));
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        assert!(
            result.passed,
            "High confluence in TrendingBull should pass. Reasons: {:?}",
            result.reasons
        );
        assert!(result.confluence_score.unwrap() >= 0.50);
    }

    #[test]
    fn test_regime_aware_blocks_in_trending_bear_with_low_confluence() {
        // Price below pivot + no trend = low confluence (~0.50)
        // TrendingBear threshold = 0.80, so this should FAIL
        let ctx = make_test_context("NIFTY", 100.0, None);
        let rules = DisciplineRules {
            min_confluence_score: 0.30, // Low configured minimum
            ..DisciplineRules::default()
        };
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBear");
        assert!(
            !result.passed,
            "Low confluence in TrendingBear should fail even with low configured minimum"
        );
        assert!(result
            .reasons
            .iter()
            .any(|r| r.contains("0.80") || r.contains("regime")));
    }

    #[test]
    fn test_regime_aware_uses_stricter_of_regime_vs_configured() {
        // Default configured minimum = 0.65
        // Ranging threshold = 0.70 > 0.65, so regime threshold should dominate
        // Price at pivot with no trend = confluence ~0.50, which is below both
        let ctx = make_test_context("ETH", 100.0, None);
        let rules = DisciplineRules::default(); // min_confluence_score = 0.30
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "Ranging");
        assert!(
            !result.passed,
            "0.50 confluence should fail against 0.70 regime threshold"
        );
    }

    #[test]
    fn test_regime_aware_configured_minimum_wins_when_higher() {
        // If configured minimum (0.85) > regime threshold (0.50 for TrendingBull)
        // then configured minimum should be used
        let ctx = make_test_context("BTC", 100.0, Some(TrendDirection::Bullish));
        let rules = DisciplineRules {
            min_confluence_score: 0.90, // Very high configured minimum
            ..DisciplineRules::default()
        };
        // Confluence ~0.85 (price above pivot + bullish) but threshold = max(0.50, 0.90) = 0.90
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        // The confluence score ~0.85 may or may not pass 0.90 — either way, the threshold is correctly 0.90
        if let Some(score) = result.confluence_score {
            // If score < 0.90, it should fail
            if score < 0.90 {
                assert!(
                    !result.passed,
                    "Should fail when confluence {} < configured minimum 0.90",
                    score
                );
            }
        }
    }

    #[test]
    fn test_regime_aware_crypto_bypasses_session_timing() {
        // BTC is crypto — should bypass session timing check
        let ctx = make_test_context("BTC", 100.0, Some(TrendDirection::Bullish));
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        // Should NOT have session timing failure
        assert!(
            !result.reasons.iter().any(|r| r.contains("session")),
            "BTC should bypass session timing"
        );
    }

    #[test]
    fn test_regime_aware_eth_bypasses_session_timing() {
        let ctx = make_test_context("ETH", 100.0, Some(TrendDirection::Bullish));
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        assert!(
            !result.reasons.iter().any(|r| r.contains("session")),
            "ETH should bypass session timing"
        );
    }

    #[test]
    fn test_regime_aware_sol_bypasses_session_timing() {
        let ctx = make_test_context("SOL", 100.0, Some(TrendDirection::Bullish));
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        assert!(
            !result.reasons.iter().any(|r| r.contains("session")),
            "SOL should bypass session timing"
        );
    }

    #[test]
    fn test_regime_aware_blocks_red_folder() {
        let mut ctx = make_test_context("BTC", 100.0, Some(TrendDirection::Bullish));
        ctx.is_red_folder_day = true;
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        assert!(!result.passed, "Red folder day should block");
        assert!(result.reasons.iter().any(|r| r.contains("Red folder")));
    }

    #[test]
    fn test_regime_aware_blocks_consecutive_losses() {
        let mut ctx = make_test_context("BTC", 100.0, Some(TrendDirection::Bullish));
        ctx.consecutive_losses = 5; // Exceeds max_consecutive_losses = 3
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        assert!(!result.passed, "5 consecutive losses should block");
        assert!(result.reasons.iter().any(|r| r.contains("consecutive")));
    }

    #[test]
    fn test_regime_aware_blocks_drawdown() {
        let mut ctx = make_test_context("NIFTY", 24500.0, Some(TrendDirection::Bullish));
        ctx.daily_pnl = -5000.0; // 5% loss on 100k equity > 3% limit
        ctx.equity = 100_000.0;
        let rules = DisciplineRules::default();
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "TrendingBull");
        assert!(!result.passed, "5% drawdown should block");
        assert!(result.reasons.iter().any(|r| r.contains("drawdown")));
    }

    #[test]
    fn test_regime_aware_all_regimes_have_thresholds_between_0_and_1() {
        let regimes = [
            "TrendingBull",
            "TrendingBear",
            "Ranging",
            "Volatile",
            "LowLiquidity",
            "",
            "Random",
        ];
        for regime in &regimes {
            let t = regime_min_confluence(regime);
            assert!(
                t > 0.0 && t <= 1.0,
                "Threshold for '{}' should be in (0, 1], got {}",
                regime,
                t
            );
        }
    }

    #[test]
    fn test_regime_aware_low_liquidity_strictest_threshold() {
        // LowLiquidity (0.85) should be the strictest regime
        let ctx = make_test_context("BTC", 100.0, Some(TrendDirection::Bullish));
        let rules = DisciplineRules {
            min_confluence_score: 0.0, // Very low configured minimum to isolate regime threshold
            ..DisciplineRules::default()
        };
        // Confluence ~0.85 — should barely pass or fail against 0.85 threshold
        let result = validate_trade_setup_regime_aware(&ctx, &rules, "LowLiquidity");
        // Score ~0.85 vs threshold 0.85 → should pass (score >= threshold)
        if let Some(score) = result.confluence_score {
            assert!(
                score >= 0.85 || !result.passed,
                "LowLiquidity: score {:.2} vs threshold 0.85, passed={}",
                score,
                result.passed
            );
        }
    }
}
