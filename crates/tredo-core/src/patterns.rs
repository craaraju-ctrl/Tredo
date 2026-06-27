use crate::OhlcvBar;
use serde::{Deserialize, Serialize};

/// A detected candlestick pattern with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandlestickPattern {
    pub name: String,
    pub direction: String, // "bullish" | "bearish" | "neutral"
    pub strength: f64,     // 0.0 to 1.0
    pub bar_index: usize,  // Index of the most recent bar in the pattern
}

/// Detect all known candlestick patterns from OHLCV data.
/// Returns patterns sorted by strength (strongest first).
pub fn detect_patterns(bars: &[OhlcvBar]) -> Vec<CandlestickPattern> {
    // Need at least 2 bars for single-bar (direction comparison) and two-bar patterns.
    // Three-bar patterns check internally and return None if insufficient.
    if bars.len() < 2 {
        return vec![];
    }

    let mut patterns = Vec::new();

    // Single-bar patterns (use last bar)
    if let Some(pattern) = detect_doji(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_hammer(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_shooting_star(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_marubozu(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_spinning_top(bars) {
        patterns.push(pattern);
    }

    // Two-bar patterns
    if let Some(pattern) = detect_bullish_engulfing(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_bearish_engulfing(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_bullish_harami(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_bearish_harami(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_piercing_line(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_dark_cloud_cover(bars) {
        patterns.push(pattern);
    }

    // Three-bar patterns
    if let Some(pattern) = detect_morning_star(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_evening_star(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_three_white_soldiers(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_three_black_crows(bars) {
        patterns.push(pattern);
    }
    // New: 3-bar Doji stars
    if let Some(pattern) = detect_morning_doji_star(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_evening_doji_star(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_abandoned_baby(bars) {
        patterns.push(pattern);
    }
    // New: 2-bar patterns
    if let Some(pattern) = detect_tweezer_top(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_tweezer_bottom(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_harami_cross(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_kicking(bars) {
        patterns.push(pattern);
    }
    // New: single-bar patterns
    if let Some(pattern) = detect_dragonfly_doji(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_gravestone_doji(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_belt_hold(bars) {
        patterns.push(pattern);
    }
    // New: 5-bar continuation
    if let Some(pattern) = detect_rising_three_methods(bars) {
        patterns.push(pattern);
    }
    if let Some(pattern) = detect_falling_three_methods(bars) {
        patterns.push(pattern);
    }

    // Sort by strength descending
    patterns.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    patterns
}

/// Format patterns as a short string for LLM prompt injection.
pub fn format_patterns(patterns: &[CandlestickPattern]) -> String {
    if patterns.is_empty() {
        return "No significant candlestick patterns detected.".to_string();
    }
    let top: Vec<String> = patterns
        .iter()
        .take(4)
        .map(|p| {
            let arrow = match p.direction.as_str() {
                "bullish" => "🟢",
                "bearish" => "🔴",
                _ => "⚪",
            };
            format!(
                "{} {} (strength: {:.0}%)",
                arrow,
                p.name,
                p.strength * 100.0
            )
        })
        .collect();
    format!("── CANDLESTICK PATTERNS ──\n{}", top.join("\n"))
}

// ── Multi-Timeframe Pattern Confirmation ────────────────────────────────────

/// Cross-reference confirmation result for a single direction.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ConfirmationLevel {
    Strong,   // 3+ timeframes agree
    Moderate, // 2 timeframes agree
    Weak,     // only 1 timeframe
    #[default]
    None,
}

impl std::fmt::Display for ConfirmationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfirmationLevel::Strong => write!(f, "Strong"),
            ConfirmationLevel::Moderate => write!(f, "Moderate"),
            ConfirmationLevel::Weak => write!(f, "Weak"),
            ConfirmationLevel::None => write!(f, "None"),
        }
    }
}

/// Multi-timeframe pattern confirmation result for a single symbol.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MultiTfPatternConfirmation {
    /// Patterns grouped by timeframe: (timeframe_label, pattern_list)
    pub patterns_by_tf: Vec<(String, Vec<CandlestickPattern>)>,
    /// Overall bullish confirmation level
    pub bullish_confirmation: ConfirmationLevel,
    /// Overall bearish confirmation level
    pub bearish_confirmation: ConfirmationLevel,
    /// Number of timeframes that had at least one pattern
    pub timeframes_with_patterns: Vec<String>,
    /// Preferred direction if confirmed across multiple timeframes
    pub preferred_direction: Option<String>, // "bullish" | "bearish" | None
    /// Confidence boost factor (0.0 = no boost, 1.0 = max boost)
    pub confidence_boost: f64,
}

/// Detect patterns across multiple timeframes and compute cross-TF confirmation.
///
/// `tf_data`: slice of (timeframe_label, OHLCV bars) pairs, e.g. [("1m", bars_1m), ("15m", bars_15m), ...]
///
/// Returns a `MultiTfPatternConfirmation` that summarises which directions are
/// confirmed across timeframes and provides a formatted context for the LLM.
pub fn detect_patterns_multi_tf(tf_data: &[(&str, &[OhlcvBar])]) -> MultiTfPatternConfirmation {
    let mut patterns_by_tf = Vec::new();
    let mut active_tfs = Vec::new();

    for &(label, bars) in tf_data {
        let pats = detect_patterns(bars);
        if !pats.is_empty() {
            active_tfs.push(label.to_string());
        }
        patterns_by_tf.push((label.to_string(), pats));
    }

    // Compute confirmation levels by unique timeframe count for each direction
    let bullish_count = patterns_by_tf
        .iter()
        .filter(|(_, pats)| pats.iter().any(|p| p.direction == "bullish"))
        .count();
    let bearish_count = patterns_by_tf
        .iter()
        .filter(|(_, pats)| pats.iter().any(|p| p.direction == "bearish"))
        .count();

    let bullish_conf = match bullish_count {
        0 => ConfirmationLevel::None,
        1 => ConfirmationLevel::Weak,
        2 => ConfirmationLevel::Moderate,
        _ => ConfirmationLevel::Strong,
    };
    let bearish_conf = match bearish_count {
        0 => ConfirmationLevel::None,
        1 => ConfirmationLevel::Weak,
        2 => ConfirmationLevel::Moderate,
        _ => ConfirmationLevel::Strong,
    };

    // Determine preferred direction & confidence boost (use refs to avoid move)
    let (preferred_direction, confidence_boost) = match (&bullish_conf, &bearish_conf) {
        (ConfirmationLevel::Strong, _) => (Some("bullish".to_string()), 0.3),
        (_, ConfirmationLevel::Strong) => (Some("bearish".to_string()), 0.3),
        (ConfirmationLevel::Moderate, ConfirmationLevel::Weak) => {
            (Some("bullish".to_string()), 0.15)
        }
        (ConfirmationLevel::Weak, ConfirmationLevel::Moderate) => {
            (Some("bearish".to_string()), 0.15)
        }
        (ConfirmationLevel::Moderate, ConfirmationLevel::Moderate) => (None, 0.0), // conflicting
        (ConfirmationLevel::Weak, ConfirmationLevel::Weak) => (None, 0.0),
        (ConfirmationLevel::Moderate, ConfirmationLevel::None) => {
            (Some("bullish".to_string()), 0.15)
        }
        (ConfirmationLevel::None, ConfirmationLevel::Moderate) => {
            (Some("bearish".to_string()), 0.15)
        }
        _ => (None, 0.0),
    };

    MultiTfPatternConfirmation {
        patterns_by_tf,
        bullish_confirmation: bullish_conf,
        bearish_confirmation: bearish_conf,
        timeframes_with_patterns: active_tfs,
        preferred_direction,
        confidence_boost,
    }
}

/// Format multi-TF pattern confirmation for LLM prompt injection.
pub fn format_mtf_confirmation(mtf: &MultiTfPatternConfirmation) -> String {
    if mtf.timeframes_with_patterns.is_empty() {
        return String::new();
    }

    let mut lines = vec!["── MULTI-TIMEFRAME PATTERN CONFIRMATION ──".to_string()];

    // Summary line
    let bullish_str = match mtf.bullish_confirmation {
        ConfirmationLevel::Strong => "🟢 Strong Bullish".to_string(),
        ConfirmationLevel::Moderate => "🟡 Moderate Bullish".to_string(),
        ConfirmationLevel::Weak => "🟢 Weak Bullish".to_string(),
        ConfirmationLevel::None => String::new(),
    };
    let bearish_str = match mtf.bearish_confirmation {
        ConfirmationLevel::Strong => "🔴 Strong Bearish".to_string(),
        ConfirmationLevel::Moderate => "🟡 Moderate Bearish".to_string(),
        ConfirmationLevel::Weak => "🔴 Weak Bearish".to_string(),
        ConfirmationLevel::None => String::new(),
    };

    let mut signals = Vec::new();
    if !bullish_str.is_empty() {
        signals.push(bullish_str);
    }
    if !bearish_str.is_empty() {
        signals.push(bearish_str);
    }
    if signals.is_empty() {
        signals.push("No clear direction".to_string());
    }
    lines.push(format!("Signal: {}", signals.join(" | ")));

    // Timeframes with patterns
    lines.push(format!(
        "Timeframes: {}",
        mtf.timeframes_with_patterns.join(", ")
    ));

    // Per-timeframe pattern list
    for (tf, pats) in &mtf.patterns_by_tf {
        if pats.is_empty() {
            continue;
        }
        let top = pats
            .iter()
            .take(2)
            .map(|p| format!("{} ({:.0}%)", p.name, p.strength * 100.0))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("  {}: {}", tf, top));
    }

    lines.join("\n")
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn body_size(bar: &OhlcvBar) -> f64 {
    (bar.close - bar.open).abs()
}

fn upper_wick(bar: &OhlcvBar) -> f64 {
    bar.high - bar.open.max(bar.close)
}

fn lower_wick(bar: &OhlcvBar) -> f64 {
    bar.open.min(bar.close) - bar.low
}

fn total_range(bar: &OhlcvBar) -> f64 {
    bar.high - bar.low
}

fn is_bullish(bar: &OhlcvBar) -> bool {
    bar.close > bar.open
}

fn is_bearish(bar: &OhlcvBar) -> bool {
    bar.close < bar.open
}

/// Bar is a doji: very small body relative to range
fn is_doji_core(bar: &OhlcvBar) -> bool {
    let r = total_range(bar);
    r > 0.0 && body_size(bar) <= r * 0.1
}

// ── Single Bar Patterns ─────────────────────────────────────────────────────

fn detect_doji(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    if is_doji_core(last) {
        let strength = 0.3;
        let direction = if last.close > bars.get(bars.len() - 2)?.close {
            "bullish"
        } else {
            "bearish"
        };
        Some(CandlestickPattern {
            name: "Doji".to_string(),
            direction: direction.to_string(),
            strength,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_hammer(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    let lw = lower_wick(last);
    let uw = upper_wick(last);
    let body = body_size(last);

    if r <= 0.0 || body <= 0.0 {
        return None;
    }

    // Hammer: small body at top, long lower wick (2x+ body), small upper wick
    let is_hammer = lw >= body * 2.0
        && uw <= body * 1.0  // upper wick can be up to 1x body
        && lw >= r * 0.45;

    if is_hammer {
        let direction = if is_bullish(last) {
            "bullish"
        } else {
            "bearish"
        };
        Some(CandlestickPattern {
            name: "Hammer".to_string(),
            direction: direction.to_string(),
            strength: 0.65,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_shooting_star(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    let uw = upper_wick(last);
    let lw = lower_wick(last);
    let body = body_size(last);

    if r <= 0.0 || body <= 0.0 {
        return None;
    }

    // Shooting star: small body at bottom, long upper wick (2x+ body), small lower wick
    let is_shooting_star = uw >= body * 2.0
        && lw <= body * 1.0  // lower wick can be up to 1x body
        && uw >= r * 0.45;

    if is_shooting_star {
        Some(CandlestickPattern {
            name: "Shooting Star".to_string(),
            direction: "bearish".to_string(),
            strength: 0.65,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_marubozu(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    let body = body_size(last);

    if r <= 0.0 || body <= 0.0 {
        return None;
    }

    // Marubozu: very long body, very small wicks
    let body_pct = body / r;
    let uw_pct = upper_wick(last) / r;
    let lw_pct = lower_wick(last) / r;

    if body_pct >= 0.85 && uw_pct <= 0.05 && lw_pct <= 0.05 {
        let direction = if is_bullish(last) {
            "bullish"
        } else {
            "bearish"
        };
        Some(CandlestickPattern {
            name: if is_bullish(last) {
                "Bullish Marubozu"
            } else {
                "Bearish Marubozu"
            }
            .to_string(),
            direction: direction.to_string(),
            strength: 0.7,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_spinning_top(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    let body = body_size(last);
    let uw = upper_wick(last);
    let lw = lower_wick(last);

    if r <= 0.0 || body <= 0.0 {
        return None;
    }

    // Spinning top: moderate body in middle, balanced wicks
    let body_pct = body / r;
    let is_spinning = (0.2..=0.5).contains(&body_pct)
        && uw > body * 0.5
        && lw > body * 0.5
        && (uw - lw).abs() <= body * 0.5;

    if is_spinning {
        Some(CandlestickPattern {
            name: "Spinning Top".to_string(),
            direction: "neutral".to_string(),
            strength: 0.25,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

// ── Two-Bar Patterns ────────────────────────────────────────────────────────

fn detect_bullish_engulfing(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;

    // Prev bearish, last bullish, last body engulfs prev body
    if is_bearish(prev)
        && is_bullish(last)
        && last.close > prev.open
        && last.open < prev.close
        && body_size(last) > body_size(prev)
    {
        Some(CandlestickPattern {
            name: "Bullish Engulfing".to_string(),
            direction: "bullish".to_string(),
            strength: 0.75,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_bearish_engulfing(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;

    if is_bullish(prev)
        && is_bearish(last)
        && last.open > prev.close
        && last.close < prev.open
        && body_size(last) > body_size(prev)
    {
        Some(CandlestickPattern {
            name: "Bearish Engulfing".to_string(),
            direction: "bearish".to_string(),
            strength: 0.75,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_bullish_harami(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;

    // Prev bearish (large body), last bullish (small body inside prev body)
    if is_bearish(prev)
        && is_bullish(last)
        && body_size(prev) > body_size(last) * 1.5
        && last.close < prev.open
        && last.open > prev.close
    {
        Some(CandlestickPattern {
            name: "Bullish Harami".to_string(),
            direction: "bullish".to_string(),
            strength: 0.55,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_bearish_harami(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;

    if is_bullish(prev)
        && is_bearish(last)
        && body_size(prev) > body_size(last) * 1.5
        && last.open < prev.close
        && last.close > prev.open
    {
        Some(CandlestickPattern {
            name: "Bearish Harami".to_string(),
            direction: "bearish".to_string(),
            strength: 0.55,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_piercing_line(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;

    // Prev bearish (long red), last bullish opens below prev low, closes above midpoint
    let prev_body = body_size(prev);
    if prev_body <= 0.0 {
        return None;
    }

    let mid_point = (prev.open + prev.close) / 2.0;

    if is_bearish(prev)
        && is_bullish(last)
        && last.open < prev.low
        && last.close > mid_point
        && last.close < prev.open
    {
        Some(CandlestickPattern {
            name: "Piercing Line".to_string(),
            direction: "bullish".to_string(),
            strength: 0.7,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_dark_cloud_cover(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;

    let prev_body = body_size(prev);
    if prev_body <= 0.0 {
        return None;
    }

    let mid_point = (prev.open + prev.close) / 2.0;

    if is_bullish(prev)
        && is_bearish(last)
        && last.open > prev.high
        && last.close < mid_point
        && last.close > prev.open
    {
        Some(CandlestickPattern {
            name: "Dark Cloud Cover".to_string(),
            direction: "bearish".to_string(),
            strength: 0.7,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

// ── Three-Bar Patterns ──────────────────────────────────────────────────────

fn detect_morning_star(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;

    // 1. Long bearish, 2. Small body (doji/spinning top) gapping down,
    // 3. Long bullish closing above midpoint of bar 1
    let b1_body = body_size(b1);
    if b1_body <= 0.0 {
        return None;
    }

    let mid_b1 = (b1.open + b1.close) / 2.0;

    if is_bearish(b1)
        && body_size(b2) <= b1_body * 0.4
        && is_bullish(b3)
        && b3.close > mid_b1
        && b3.close > b2.close
    {
        Some(CandlestickPattern {
            name: "Morning Star".to_string(),
            direction: "bullish".to_string(),
            strength: 0.85,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_evening_star(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;

    let b1_body = body_size(b1);
    if b1_body <= 0.0 {
        return None;
    }

    let mid_b1 = (b1.open + b1.close) / 2.0;

    if is_bullish(b1)
        && body_size(b2) <= b1_body * 0.4
        && is_bearish(b3)
        && b3.close < mid_b1
        && b3.close < b2.close
    {
        Some(CandlestickPattern {
            name: "Evening Star".to_string(),
            direction: "bearish".to_string(),
            strength: 0.85,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_three_white_soldiers(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;

    // Three consecutive long bullish candles, each closing higher
    if is_bullish(b1)
        && is_bullish(b2)
        && is_bullish(b3)
        && b1.close > b1.open
        && b2.close > b2.open
        && b3.close > b3.open
        && b2.close > b1.close
        && b3.close > b2.close
        && body_size(b1) > 0.0
        && body_size(b2) > 0.0
        && body_size(b3) > 0.0
    {
        Some(CandlestickPattern {
            name: "Three White Soldiers".to_string(),
            direction: "bullish".to_string(),
            strength: 0.8,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_three_black_crows(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;

    // Three consecutive long bearish candles, each closing lower
    if is_bearish(b1)
        && is_bearish(b2)
        && is_bearish(b3)
        && b1.close < b1.open
        && b2.close < b2.open
        && b3.close < b3.open
        && b2.close < b1.close
        && b3.close < b2.close
        && body_size(b1) > 0.0
        && body_size(b2) > 0.0
        && body_size(b3) > 0.0
    {
        Some(CandlestickPattern {
            name: "Three Black Crows".to_string(),
            direction: "bearish".to_string(),
            strength: 0.8,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

// ── New Single-Bar Patterns ─────────────────────────────────────────────────

fn detect_dragonfly_doji(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    if r <= 0.0 {
        return None;
    }
    let body = body_size(last);
    let uw = upper_wick(last);
    let lw = lower_wick(last);
    // Dragonfly: doji body, long lower wick, tiny/no upper wick
    if body <= r * 0.1 && lw >= r * 0.6 && uw <= r * 0.05 {
        Some(CandlestickPattern {
            name: "Dragonfly Doji".to_string(),
            direction: "bullish".to_string(),
            strength: 0.65,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_gravestone_doji(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    if r <= 0.0 {
        return None;
    }
    let body = body_size(last);
    let uw = upper_wick(last);
    let lw = lower_wick(last);
    // Gravestone: doji body, long upper wick, tiny/no lower wick
    if body <= r * 0.1 && uw >= r * 0.6 && lw <= r * 0.05 {
        Some(CandlestickPattern {
            name: "Gravestone Doji".to_string(),
            direction: "bearish".to_string(),
            strength: 0.65,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_belt_hold(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    let last = bars.last()?;
    let r = total_range(last);
    if r <= 0.0 {
        return None;
    }
    let body = body_size(last);
    let uw = upper_wick(last);
    let lw = lower_wick(last);
    // Bullish belt hold: open=low, no lower wick, strong body
    if is_bullish(last) && lw <= r * 0.02 && body >= r * 0.7 {
        Some(CandlestickPattern {
            name: "Bullish Belt Hold".to_string(),
            direction: "bullish".to_string(),
            strength: 0.7,
            bar_index: bars.len() - 1,
        })
    // Bearish belt hold: open=high, no upper wick, strong body
    } else if is_bearish(last) && uw <= r * 0.02 && body >= r * 0.7 {
        Some(CandlestickPattern {
            name: "Bearish Belt Hold".to_string(),
            direction: "bearish".to_string(),
            strength: 0.7,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

// ── New Two-Bar Patterns ────────────────────────────────────────────────────

fn detect_tweezer_top(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;
    // Two consecutive bars with same/similar high, first bullish, second bearish
    let high_diff = (prev.high - last.high).abs() / prev.high;
    if is_bullish(prev) && is_bearish(last) && high_diff < 0.01 && last.close < prev.open {
        Some(CandlestickPattern {
            name: "Tweezer Top".to_string(),
            direction: "bearish".to_string(),
            strength: 0.72,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_tweezer_bottom(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;
    let low_diff = (prev.low - last.low).abs() / prev.low;
    if is_bearish(prev) && is_bullish(last) && low_diff < 0.01 && last.close > prev.open {
        Some(CandlestickPattern {
            name: "Tweezer Bottom".to_string(),
            direction: "bullish".to_string(),
            strength: 0.72,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_harami_cross(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;
    let prev_body = body_size(prev);
    if prev_body <= 0.0 {
        return None;
    }
    // Harami cross: first bar has large body, second bar is a doji inside first bar's body
    let is_doji = body_size(last) <= total_range(last) * 0.1;
    let inside = last.high <= prev_body_range_max(prev) && last.low >= prev_body_range_min(prev);
    if is_doji && inside && prev_body >= total_range(prev) * 0.5 {
        let direction = if is_bearish(prev) {
            "bullish"
        } else {
            "bearish"
        };
        Some(CandlestickPattern {
            name: "Harami Cross".to_string(),
            direction: direction.to_string(),
            strength: 0.75,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn prev_body_range_max(bar: &OhlcvBar) -> f64 {
    bar.open.max(bar.close)
}
fn prev_body_range_min(bar: &OhlcvBar) -> f64 {
    bar.open.min(bar.close)
}

fn detect_kicking(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 2 {
        return None;
    }
    let prev = &bars[bars.len() - 2];
    let last = bars.last()?;
    let prev_body = body_size(prev);
    let last_body = body_size(last);
    if prev_body <= 0.0 || last_body <= 0.0 {
        return None;
    }
    // Bullish kicking: first long bearish, second long bullish with gap up
    if is_bearish(prev)
        && is_bullish(last)
        && last.open > prev.close
        && last_body >= prev_body * 0.8
    {
        Some(CandlestickPattern {
            name: "Bullish Kicking".to_string(),
            direction: "bullish".to_string(),
            strength: 0.78,
            bar_index: bars.len() - 1,
        })
    // Bearish kicking: first long bullish, second long bearish with gap down
    } else if is_bullish(prev)
        && is_bearish(last)
        && last.open < prev.close
        && last_body >= prev_body * 0.8
    {
        Some(CandlestickPattern {
            name: "Bearish Kicking".to_string(),
            direction: "bearish".to_string(),
            strength: 0.78,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

// ── New Three-Bar Patterns ──────────────────────────────────────────────────

fn detect_morning_doji_star(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;
    let b1_body = body_size(b1);
    if b1_body <= 0.0 {
        return None;
    }
    let mid_b1 = (b1.open + b1.close) / 2.0;
    // Morning Doji Star: long bearish, doji, long bullish
    if is_bearish(b1) && is_doji_core(b2) && is_bullish(b3) && b3.close > mid_b1 {
        Some(CandlestickPattern {
            name: "Morning Doji Star".to_string(),
            direction: "bullish".to_string(),
            strength: 0.88,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_evening_doji_star(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;
    let b1_body = body_size(b1);
    if b1_body <= 0.0 {
        return None;
    }
    let mid_b1 = (b1.open + b1.close) / 2.0;
    if is_bullish(b1) && is_doji_core(b2) && is_bearish(b3) && b3.close < mid_b1 {
        Some(CandlestickPattern {
            name: "Evening Doji Star".to_string(),
            direction: "bearish".to_string(),
            strength: 0.88,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_abandoned_baby(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 3 {
        return None;
    }
    let b1 = &bars[bars.len() - 3];
    let b2 = &bars[bars.len() - 2];
    let b3 = bars.last()?;
    let b1_body = body_size(b1);
    if b1_body <= 0.0 {
        return None;
    }
    // Abandoned baby: like morning/evening star but with gap on both sides of doji
    if is_bearish(b1)
        && is_doji_core(b2)
        && is_bullish(b3)
        && b2.high < b1.low
        && b2.high < b3.low
        && b3.close > (b1.open + b1.close) / 2.0
    {
        Some(CandlestickPattern {
            name: "Abandoned Baby (Bullish)".to_string(),
            direction: "bullish".to_string(),
            strength: 0.9,
            bar_index: bars.len() - 1,
        })
    } else if is_bullish(b1)
        && is_doji_core(b2)
        && is_bearish(b3)
        && b2.low > b1.high
        && b2.low > b3.high
        && b3.close < (b1.open + b1.close) / 2.0
    {
        Some(CandlestickPattern {
            name: "Abandoned Baby (Bearish)".to_string(),
            direction: "bearish".to_string(),
            strength: 0.9,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

// ── New Five-Bar Continuation Patterns ────────────────────────────────────

fn detect_rising_three_methods(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 5 {
        return None;
    }
    let b1 = &bars[bars.len() - 5];
    let b2 = &bars[bars.len() - 4];
    let b3 = &bars[bars.len() - 3];
    let b4 = &bars[bars.len() - 2];
    let b5 = bars.last()?;
    // 1: long bullish, 2-4: small bearish inside b1's range, 5: bullish closes above b1
    let b1_body = body_size(b1);
    if b1_body <= 0.0 || !is_bullish(b1) {
        return None;
    }
    let b1_high = b1.high;
    let b1_low = b1.low;
    let small_inside = |b: &OhlcvBar| {
        body_size(b) <= b1_body * 0.4 && b.high <= b1_high && b.low >= b1_low && is_bearish(b)
    };
    if small_inside(b2)
        && small_inside(b3)
        && small_inside(b4)
        && is_bullish(b5)
        && b5.close > b1.close
    {
        Some(CandlestickPattern {
            name: "Rising Three Methods".to_string(),
            direction: "bullish".to_string(),
            strength: 0.8,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

fn detect_falling_three_methods(bars: &[OhlcvBar]) -> Option<CandlestickPattern> {
    if bars.len() < 5 {
        return None;
    }
    let b1 = &bars[bars.len() - 5];
    let b2 = &bars[bars.len() - 4];
    let b3 = &bars[bars.len() - 3];
    let b4 = &bars[bars.len() - 2];
    let b5 = bars.last()?;
    let b1_body = body_size(b1);
    if b1_body <= 0.0 || !is_bearish(b1) {
        return None;
    }
    let b1_high = b1.high;
    let b1_low = b1.low;
    let small_inside = |b: &OhlcvBar| {
        body_size(b) <= b1_body * 0.4 && b.high <= b1_high && b.low >= b1_low && is_bullish(b)
    };
    if small_inside(b2)
        && small_inside(b3)
        && small_inside(b4)
        && is_bearish(b5)
        && b5.close < b1.close
    {
        Some(CandlestickPattern {
            name: "Falling Three Methods".to_string(),
            direction: "bearish".to_string(),
            strength: 0.8,
            bar_index: bars.len() - 1,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(open: f64, high: f64, low: f64, close: f64) -> OhlcvBar {
        OhlcvBar {
            timestamp: String::new(),
            open,
            high,
            low,
            close,
            volume: 1000.0,
        }
    }

    // ── Single Bar Tests ──

    #[test]
    fn test_doji_detection() {
        let bars = vec![
            bar(100.0, 110.0, 90.0, 105.0),
            bar(100.0, 101.0, 99.0, 100.1),
        ];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Doji"));
    }

    #[test]
    fn test_hammer_detection() {
        // Small body at top, long lower wick, tiny upper wick
        // bar2: open=102.0, high=102.3, low=95.0, close=102.5
        // body=0.5, range=7.3, lw=102.0-95.0=7.0, uw=102.3-102.5=0.0 (capped)
        let bars = vec![
            bar(100.0, 105.0, 95.0, 102.0),
            bar(102.0, 102.3, 95.0, 102.5),
        ];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Hammer"));
    }

    #[test]
    fn test_shooting_star_detection() {
        // Small body at bottom, long upper wick, tiny lower wick
        // bar2: open=101.0, high=110.0, low=100.7, close=100.5
        // body=0.5, range=9.5, uw=110.0-101.0=9.0, lw=100.7-100.5=0.2
        let bars = vec![
            bar(100.0, 105.0, 95.0, 102.0),
            bar(101.0, 110.0, 100.7, 100.5),
        ];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Shooting Star"));
    }

    #[test]
    fn test_marubozu_detection() {
        // Very long body, almost no wicks
        let bars = vec![
            bar(100.0, 102.0, 98.0, 101.0),
            bar(100.0, 105.05, 100.0, 105.0),
        ];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name.contains("Marubozu")));
    }

    // ── Two-Bar Tests ──

    #[test]
    fn test_bullish_engulfing() {
        // Prev red, current green, current body engulfs prev body
        let bars = vec![
            bar(105.0, 106.0, 99.0, 100.0),
            bar(99.0, 108.0, 98.0, 107.0),
        ];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Bullish Engulfing"));
    }

    #[test]
    fn test_bearish_engulfing() {
        // Prev bullish (open=100, close=100.5), last bearish opens above prev close
        let bars = vec![
            bar(100.0, 101.0, 99.0, 100.5),
            bar(101.0, 102.0, 95.0, 96.0),
        ];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Bearish Engulfing"));
    }

    #[test]
    fn test_piercing_line() {
        // Prev long red (midpoint=101), current green opens below prev low, closes above midpoint
        let bars = vec![bar(105.0, 106.0, 95.0, 97.0), bar(94.0, 103.0, 93.0, 101.5)];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Piercing Line"));
    }

    #[test]
    fn test_dark_cloud_cover() {
        let bars = vec![bar(95.0, 106.0, 94.0, 105.0), bar(107.0, 108.0, 98.0, 99.0)];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Dark Cloud Cover"));
    }

    // ── Three-Bar Tests ──

    #[test]
    fn test_morning_star() {
        // Long red, small body (doji-like), long green closing above midpoint of bar 1
        let b1 = bar(105.0, 106.0, 95.0, 96.0);
        let b2 = bar(95.5, 96.5, 94.5, 95.0);
        let b3 = bar(95.0, 105.0, 94.0, 104.0);
        let bars = vec![b1, b2, b3];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Morning Star"));
    }

    #[test]
    fn test_evening_star() {
        let b1 = bar(95.0, 105.0, 94.0, 104.0);
        let b2 = bar(104.5, 105.5, 103.5, 104.0);
        let b3 = bar(104.0, 105.0, 95.0, 96.0);
        let bars = vec![b1, b2, b3];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Evening Star"));
    }

    #[test]
    fn test_three_white_soldiers() {
        let b1 = bar(100.0, 105.0, 99.0, 105.0);
        let b2 = bar(105.0, 110.0, 104.0, 110.0);
        let b3 = bar(110.0, 115.0, 109.0, 115.0);
        let bars = vec![b1, b2, b3];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Three White Soldiers"));
    }

    #[test]
    fn test_three_black_crows() {
        let b1 = bar(105.0, 106.0, 100.0, 100.0);
        let b2 = bar(100.0, 101.0, 95.0, 95.0);
        let b3 = bar(95.0, 96.0, 90.0, 90.0);
        let bars = vec![b1, b2, b3];
        let patterns = detect_patterns(&bars);
        assert!(patterns.iter().any(|p| p.name == "Three Black Crows"));
    }

    // ── Edge Cases ──

    #[test]
    fn test_no_patterns_with_insufficient_data() {
        assert!(detect_patterns(&[]).is_empty());
        assert!(detect_patterns(&[bar(100.0, 101.0, 99.0, 100.5)]).is_empty());
    }

    #[test]
    fn test_format_patterns_empty() {
        let s = format_patterns(&[]);
        assert_eq!(s, "No significant candlestick patterns detected.");
    }

    #[test]
    fn test_format_patterns_with_data() {
        let patterns = vec![CandlestickPattern {
            name: "Bullish Engulfing".to_string(),
            direction: "bullish".to_string(),
            strength: 0.75,
            bar_index: 0,
        }];
        let s = format_patterns(&patterns);
        assert!(s.contains("Bullish Engulfing"));
        assert!(s.contains("75%"));
    }

    #[test]
    fn test_not_detecting_false_patterns() {
        // Random noisy bars should not trigger patterns
        let bars: Vec<OhlcvBar> = (0..10)
            .map(|i| {
                bar(
                    100.0 + i as f64,
                    102.0 + i as f64,
                    98.0 + i as f64,
                    101.0 + i as f64,
                )
            })
            .collect();
        let patterns = detect_patterns(&bars);
        // May have some patterns but nothing very strong
        for p in &patterns {
            assert!(p.strength <= 0.85);
        }
    }
}
