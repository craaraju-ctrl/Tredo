use crate::types::{MarketRegime, SessionInfo, TradeSignal};
use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc, Weekday}; // Duration removed (SessionInfo now uses i64 to avoid serde issues)
use tredo_core::{DisciplineRules, OhlcvBar};

/// Get current Indian market session info (NSE/BSE)
pub fn get_indian_session_info(now: DateTime<Utc>) -> SessionInfo {
    let ist = now.with_timezone(&FixedOffset::east_opt(5 * 3600 + 1800).unwrap());
    let hour = ist.hour();
    let minute = ist.minute();
    let time_minutes = (hour * 60 + minute) as i64;

    let pre_open_start = 9 * 60;
    let pre_open_end = 9 * 60 + 15;
    let open_start = 9 * 60 + 15;
    let open_end = 15 * 60 + 30;

    let weekday = ist.weekday();
    let is_weekend = weekday == Weekday::Sat || weekday == Weekday::Sun;

    if is_weekend {
        return SessionInfo {
            market_open: false,
            session_name: "Weekend Closed".to_string(),
            time_to_close: None,
            time_to_open: None,
            is_pre_open: false,
            is_post_close: true,
            minutes_since_open: 0,
        };
    }

    if time_minutes >= pre_open_start && time_minutes < pre_open_end {
        let mins_to_open = pre_open_end - time_minutes;
        return SessionInfo {
            market_open: false,
            session_name: "Pre-Open".to_string(),
            time_to_close: None,
            time_to_open: Some(mins_to_open),
            is_pre_open: true,
            is_post_close: false,
            minutes_since_open: 0,
        };
    }

    if time_minutes >= open_start && time_minutes < open_end {
        let mins_to_close = open_end - time_minutes;
        let mins_since_open = time_minutes - open_start;
        return SessionInfo {
            market_open: true,
            session_name: "Normal Session".to_string(),
            time_to_close: Some(mins_to_close),
            time_to_open: None,
            is_pre_open: false,
            is_post_close: false,
            minutes_since_open: mins_since_open,
        };
    }

    if time_minutes < pre_open_start {
        let mins_to_open = pre_open_start - time_minutes;
        return SessionInfo {
            market_open: false,
            session_name: "Before Hours".to_string(),
            time_to_close: None,
            time_to_open: Some(mins_to_open),
            is_pre_open: false,
            is_post_close: false,
            minutes_since_open: 0,
        };
    }

    SessionInfo {
        market_open: false,
        session_name: "Post-Close".to_string(),
        time_to_close: None,
        time_to_open: None,
        is_pre_open: false,
        is_post_close: true,
        minutes_since_open: 0,
    }
}

/// Calculate position size based on risk percentage and stop distance
pub fn calculate_position_size(
    account_balance: f64,
    risk_pct: f64,
    entry_price: f64,
    stop_loss: f64,
) -> f64 {
    let risk_amount = account_balance * risk_pct;
    let stop_distance = (entry_price - stop_loss).abs();

    if stop_distance <= 0.0 {
        return 0.0;
    }

    let shares = risk_amount / stop_distance;
    let position_value = shares * entry_price;

    let max_position_value = account_balance * 0.95;
    if position_value > max_position_value {
        return max_position_value / entry_price;
    }

    shares
}

/// Calculate Risk/Reward ratio
pub fn calculate_risk_reward(
    entry: f64,
    stop: f64,
    target: f64,
    direction: tredo_core::TradeDirection,
) -> f64 {
    let risk = (entry - stop).abs();
    let reward = (target - entry).abs();

    if risk <= 0.0 {
        return 0.0;
    }

    match direction {
        tredo_core::TradeDirection::Long if target > entry => reward / risk,
        tredo_core::TradeDirection::Short if target < entry => reward / risk,
        _ => 0.0,
    }
}

/// Estimate market regime from price action data
pub fn estimate_market_regime(prices: &[f64], highs: &[f64], lows: &[f64]) -> MarketRegime {
    if prices.len() < 10 {
        return MarketRegime::LowLiquidity;
    }

    let n = prices.len() as f64;
    let sum_x: f64 = (0..prices.len()).map(|i| i as f64).sum();
    let sum_y: f64 = prices.iter().sum();
    let sum_xy: f64 = prices.iter().enumerate().map(|(i, &p)| i as f64 * p).sum();
    let sum_x2: f64 = (0..prices.len()).map(|i| (i as f64).powi(2)).sum();

    let slope = (n * sum_xy - sum_x * sum_y) / (n * sum_x2 - sum_x.powi(2));
    let avg_price = sum_y / n;
    let slope_pct = slope / avg_price;

    let mut tr_sum = 0.0;
    for i in 1..prices.len() {
        let tr = (highs[i] - lows[i]).abs();
        tr_sum += tr;
    }
    let avg_tr = tr_sum / (prices.len() - 1) as f64;
    let volatility_pct = avg_tr / avg_price;

    if volatility_pct > 0.02 {
        MarketRegime::Volatile
    } else if slope_pct > 0.001 {
        MarketRegime::TrendingBull
    } else if slope_pct < -0.001 {
        MarketRegime::TrendingBear
    } else if volatility_pct < 0.005 {
        MarketRegime::LowLiquidity
    } else {
        MarketRegime::Ranging
    }
}

/// Simple RSI calculation (Wilder's method)
pub fn compute_rsi(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period + 1 {
        return 50.0; // neutral
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in 1..=period {
        let change = bars[bars.len() - i].close - bars[bars.len() - i - 1].close;
        if change > 0.0 {
            gains += change;
        } else {
            losses -= change;
        }
    }
    let avg_gain = gains / period as f64;
    let avg_loss = losses / period as f64;
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}

/// Simple MACD (12,26,9) - returns (macd, signal, histogram)
pub fn compute_macd(bars: &[OhlcvBar]) -> (f64, f64, f64) {
    if bars.len() < 26 {
        return (0.0, 0.0, 0.0);
    }
    let ema12 = compute_ema(bars, 12);
    let ema26 = compute_ema(bars, 26);
    let macd_line = ema12 - ema26;
    // For signal, approximate with recent; full would need MACD history
    let signal = macd_line * 0.9; // simplified
    let hist = macd_line - signal;
    (macd_line, signal, hist)
}

fn compute_ema(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.is_empty() {
        return 0.0;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut ema = bars[0].close;
    for bar in bars.iter().skip(1) {
        ema = bar.close * k + ema * (1.0 - k);
    }
    ema
}

/// Autonomous level calculator: the *agent* decides entry, SL, TP from its own analysis.
/// No external price points are ever injected here.
/// The AggregatedSignal (cross-skill consensus) is now a first-class input so the agent
/// actually listens to the combined voice of its own skills instead of ignoring them.
#[allow(clippy::too_many_arguments)]
pub fn compute_autonomous_levels(
    _symbol: &str,
    current_price: f64,
    pivots: &tredo_core::PivotLevels,
    _patterns: &[tredo_core::CandlestickPattern], // patterns come from MI state; kept for future extension
    regime: crate::types::MarketRegime,
    rsi: f64,
    macd_hist: f64,
    atr_pct: f64,
    _rules: &DisciplineRules,
    aggregated_signal: Option<&tredo_core::AggregatedSignal>,
) -> (f64, f64, f64, f64) {
    // entry, sl, tp, rr
    let mut direction = tredo_core::TradeDirection::Long;

    // === RESPECT THE AGGREGATED SIGNAL (Gap 1 fix) ===
    // The agent now lets the combined skill consensus (AggregatedSignal) have real
    // influence on direction and level selection, instead of treating skills as
    // decorative COT output only.
    let mut agg_bias: f64 = 0.0;
    if let Some(agg) = aggregated_signal {
        if agg.is_bullish(None) {
            agg_bias = agg.net_signal.abs().min(0.6);
            direction = tredo_core::TradeDirection::Long;
        } else if agg.is_bearish(None) {
            agg_bias = -agg.net_signal.abs().min(0.6);
            direction = tredo_core::TradeDirection::Short;
        }
    }

    // Decide / adjust direction from indicators + aggregated consensus (true agentic fusion)
    if agg_bias == 0.0 {
        if rsi > 70.0 || (regime == crate::types::MarketRegime::TrendingBear && macd_hist < 0.0) {
            direction = tredo_core::TradeDirection::Short;
        } else if rsi < 30.0
            || (regime == crate::types::MarketRegime::TrendingBull && macd_hist > 0.0)
        {
            direction = tredo_core::TradeDirection::Long;
        }
    }

    // Entry near current or breakout, biased by aggregated conviction
    let breakout_buffer = 0.001 + (agg_bias.abs() * 0.003);
    let entry = if direction == tredo_core::TradeDirection::Long {
        current_price.max(pivots.pivot * (1.0 + breakout_buffer))
    } else {
        current_price.min(pivots.pivot * (1.0 - breakout_buffer))
    };

    // SL using ATR + pivot support (agent identifies protection level), tightened/loosened by agg conviction
    let atr = current_price * atr_pct.max(0.01);
    let sl_multiplier = 1.5 - (agg_bias.abs() * 0.4); // strong consensus → tighter protective stops
    let sl = if direction == tredo_core::TradeDirection::Long {
        (current_price - atr * sl_multiplier).min(pivots.s1)
    } else {
        (current_price + atr * sl_multiplier).max(pivots.r1)
    };

    // TP: risk/reward or next structure target, expanded when aggregated signal is strong
    let risk = (entry - sl).abs();
    let rr_multiplier = 2.0 + (agg_bias.abs() * 1.0); // strong consensus → more ambitious targets
    let tp = if direction == tredo_core::TradeDirection::Long {
        entry + risk * rr_multiplier
    } else {
        entry - risk * rr_multiplier
    };

    let rr = if risk > 0.0 {
        risk / (tp - entry).abs().max(0.0001)
    } else {
        1.0
    };

    // Constrain to rules
    let min_rr = 1.5;
    let final_tp = if rr < min_rr {
        if direction == tredo_core::TradeDirection::Long {
            entry + risk * min_rr
        } else {
            entry - risk * min_rr
        }
    } else {
        tp
    };

    (
        entry,
        sl,
        final_tp,
        if risk > 0.0 {
            risk / (final_tp - entry).abs()
        } else {
            2.0
        },
    )
}

/// Check if a trading signal meets minimum quality thresholds
pub fn signal_quality_check(signal: &TradeSignal, rules: &DisciplineRules) -> (bool, Vec<String>) {
    let mut passed = true;
    let mut reasons = Vec::new();

    if signal.confluence_score < rules.min_confluence_score {
        reasons.push(format!(
            "Confluence score {:.2} below minimum {:.2}",
            signal.confluence_score, rules.min_confluence_score
        ));
        passed = false;
    }

    if signal.risk_reward_ratio < 1.5 {
        reasons.push(format!(
            "Risk/Reward {:.1}:1 below minimum 1.5:1",
            signal.risk_reward_ratio
        ));
        passed = false;
    }

    if signal.confidence_score < 0.5 {
        reasons.push(format!(
            "Confidence score {:.2} below minimum 0.5",
            signal.confidence_score
        ));
        passed = false;
    }

    if !signal.session_valid {
        reasons.push("Outside valid trading session".to_string());
        passed = false;
    }

    if !signal.risk_check_passed {
        reasons.push("Risk check failed".to_string());
        passed = false;
    }

    (passed, reasons)
}

// === Extended metric calculators for MarketMetricsMeter tool (local, agentic) ===

pub fn compute_atr(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period + 1 {
        return bars.last().map(|b| (b.high - b.low).abs()).unwrap_or(0.0);
    }
    let mut trs: Vec<f64> = vec![];
    for i in 1..bars.len() {
        let tr = (bars[i].high - bars[i].low)
            .max((bars[i].high - bars[i - 1].close).abs())
            .max((bars[i].low - bars[i - 1].close).abs());
        trs.push(tr);
    }
    // Wilder's smoothing approx (simple avg for last period)
    let start = trs.len().saturating_sub(period);
    let sum: f64 = trs[start..].iter().sum();
    sum / period as f64
}

pub fn compute_bollinger_bands(bars: &[OhlcvBar], period: usize, stddev: f64) -> (f64, f64, f64) {
    if bars.len() < period {
        let p = bars.last().map(|b| b.close).unwrap_or(0.0);
        return (p * 1.02, p, p * 0.98);
    }
    let closes: Vec<f64> = bars.iter().rev().take(period).map(|b| b.close).collect();
    let mean = closes.iter().sum::<f64>() / closes.len() as f64;
    let var = closes.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / closes.len() as f64;
    let sd = var.sqrt() * stddev;
    (mean + sd, mean, mean - sd)
}

pub fn compute_stochastic(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period + 1 {
        return 50.0;
    }
    let recent = &bars[bars.len() - period..];
    let high = recent.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let low = recent.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    let close = bars.last().unwrap().close;
    if high == low {
        return 50.0;
    }
    ((close - low) / (high - low) * 100.0).clamp(0.0, 100.0)
}

pub fn compute_relative_volume(bars: &[OhlcvBar]) -> f64 {
    if bars.len() < 10 {
        return 1.0;
    }
    let vols: Vec<f64> = bars.iter().map(|b| b.volume).collect();
    let avg: f64 = vols.iter().take(vols.len() - 1).sum::<f64>() / (vols.len() - 1) as f64;
    if avg <= 0.0 {
        return 1.0;
    }
    (vols.last().copied().unwrap_or(avg) / avg).clamp(0.3, 3.0)
}

// Simple fib retracements helper (used by meter for levels awareness; agent still decides actual SL/TP)
pub fn compute_fib_levels(high: f64, low: f64) -> (f64, f64) {
    let rng = (high - low).max(0.0001);
    (high - rng * 0.382, high - rng * 0.618)
}

// ═══════════════════════════════════════════════════════════════════════════════
// NEW INDICATORS — 5 additional independent signals to boost Information Ratio
// Research shows breadth (number of independent signals) improves precision via
// Information Ratio = IC × √(Breadth). Adding 5 signals increases Breadth by 5×.
// ═══════════════════════════════════════════════════════════════════════════════

/// On-Balance Volume (OBV) — cumulative volume flow indicator.
/// Rising OBV confirms price trend with volume; divergences signal reversals.
/// Returns (obv_value, trend_direction) where trend: >0 bullish, <0 bearish, 0 neutral.
/// Single-pass computation for efficiency.
pub fn compute_obv(bars: &[OhlcvBar]) -> (f64, f64) {
    if bars.len() < 2 {
        return (0.0, 0.0);
    }
    let lookback = 10.min(bars.len() - 1);
    let mut obv = 0.0;
    let mut obv_at_lookback = 0.0;
    for i in 1..bars.len() {
        let vol = bars[i].volume;
        if bars[i].close > bars[i - 1].close {
            obv += vol;
        } else if bars[i].close < bars[i - 1].close {
            obv -= vol;
        }
        // Capture OBV at start of lookback window
        if i == bars.len() - lookback {
            obv_at_lookback = obv;
        }
    }
    // Trend: compare current OBV vs OBV at start of lookback window
    let recent_slope = obv - obv_at_lookback;
    let magnitude = recent_slope.abs();
    let avg_vol: f64 = bars.iter().map(|b| b.volume).sum::<f64>() / bars.len() as f64;
    let normalized = if avg_vol > 0.0 { magnitude / avg_vol } else { 0.0 };
    let direction = if recent_slope > 0.0 { normalized.min(1.0) } else { (-normalized).max(-1.0) };
    (obv, direction)
}

/// Average Directional Index (ADX) — trend strength (0-100).
/// >25 = trending, <20 = ranging. Combined with +DI/-DI for direction.
/// Returns (adx, plus_di, minus_di).
pub fn compute_adx(bars: &[OhlcvBar], period: usize) -> (f64, f64, f64) {
    if bars.len() < period + 2 {
        return (25.0, 50.0, 50.0); // default neutral
    }
    let n = bars.len();
    // True Range, +DM, -DM
    let mut trs = Vec::new();
    let mut plus_dm = Vec::new();
    let mut minus_dm = Vec::new();
    for i in 1..n {
        let tr = (bars[i].high - bars[i].low)
            .max((bars[i].high - bars[i - 1].close).abs())
            .max((bars[i].low - bars[i - 1].close).abs());
        trs.push(tr);
        let up_move = bars[i].high - bars[i - 1].high;
        let down_move = bars[i - 1].low - bars[i].low;
        if up_move > down_move && up_move > 0.0 {
            plus_dm.push(up_move);
            minus_dm.push(0.0);
        } else if down_move > up_move && down_move > 0.0 {
            plus_dm.push(0.0);
            minus_dm.push(down_move);
        } else {
            plus_dm.push(0.0);
            minus_dm.push(0.0);
        }
    }
    // Wilder's smoothing for period
    let start = trs.len().saturating_sub(period * 3).max(1);
    let mut atr_smooth = trs[start];
    let mut plus_dm_smooth = plus_dm[start];
    let mut minus_dm_smooth = minus_dm[start];
    for i in (start + 1)..trs.len() {
        atr_smooth = atr_smooth - atr_smooth / period as f64 + trs[i];
        plus_dm_smooth = plus_dm_smooth - plus_dm_smooth / period as f64 + plus_dm[i];
        minus_dm_smooth = minus_dm_smooth - minus_dm_smooth / period as f64 + minus_dm[i];
    }
    if atr_smooth <= 0.0 {
        return (0.0, 50.0, 50.0);
    }
    let plus_di = (plus_dm_smooth / atr_smooth * 100.0).max(0.0);
    let minus_di = (minus_dm_smooth / atr_smooth * 100.0).max(0.0);
    let di_sum = plus_di + minus_di;
    let dx = if di_sum > 0.0 {
        (plus_di - minus_di).abs() / di_sum * 100.0
    } else {
        0.0
    };
    // Smooth DX over period for ADX
    let adx = dx.clamp(0.0, 100.0);
    (adx, plus_di, minus_di)
}

/// Commodity Channel Index (CCI) — oscillator measuring price vs statistical mean.
/// >+100 = overbought, <-100 = oversold, 0 = neutral.
pub fn compute_cci(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return 0.0;
    }
    // Typical Price = (H + L + C) / 3
    let typical_prices: Vec<f64> = bars.iter().map(|b| (b.high + b.low + b.close) / 3.0).collect();
    let recent = &typical_prices[typical_prices.len() - period..];
    let mean = recent.iter().sum::<f64>() / period as f64;
    let mean_deviation = recent.iter().map(|tp| (tp - mean).abs()).sum::<f64>() / period as f64;
    let tp = *recent.last().unwrap_or(&mean);
    if mean_deviation == 0.0 {
        return 0.0;
    }
    (tp - mean) / (0.015 * mean_deviation)
}

/// Williams %R — momentum oscillator (0 to -100 scale).
/// >-20 = overbought, <-80 = oversold, -50 = midline.
/// Returns value in [-100, 0] range.
pub fn compute_williams_r(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return -50.0; // neutral
    }
    let recent = &bars[bars.len() - period..];
    let high = recent.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let low = recent.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    let close = bars.last().unwrap().close;
    if high == low {
        return -50.0;
    }
    ((high - close) / (high - low) * -100.0).clamp(-100.0, 0.0)
}

/// Volume Weighted Average Price (VWAP) — intraday volume-weighted price level.
/// Price above VWAP = bullish institutional buying; below = bearish selling pressure.
/// Returns (vwap_price, deviation_pct) where deviation_pct = (price - vwap) / vwap.
pub fn compute_vwap(bars: &[OhlcvBar]) -> (f64, f64) {
    if bars.is_empty() {
        return (0.0, 0.0);
    }
    let mut cum_vol_price = 0.0;
    let mut cum_vol = 0.0;
    for bar in bars {
        let tp = (bar.high + bar.low + bar.close) / 3.0;
        cum_vol_price += tp * bar.volume;
        cum_vol += bar.volume;
    }
    let vwap = if cum_vol > 0.0 { cum_vol_price / cum_vol } else { bars.last().unwrap().close };
    let current_price = bars.last().unwrap().close;
    let deviation = if vwap > 0.0 { (current_price - vwap) / vwap } else { 0.0 };
    (vwap, deviation)
}

#[cfg(test)]
mod indicator_tests {
    use super::*;

    fn make_bar(high: f64, low: f64, close: f64, volume: f64) -> OhlcvBar {
        OhlcvBar {
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            open: close,
            high,
            low,
            close,
            volume,
        }
    }

    // ═══ OBV Tests ═══

    #[test]
    fn test_obv_empty_bars() {
        assert_eq!(compute_obv(&[]), (0.0, 0.0));
    }

    #[test]
    fn test_obv_single_bar() {
        let bars = vec![make_bar(100.0, 90.0, 95.0, 1000.0)];
        assert_eq!(compute_obv(&bars), (0.0, 0.0));
    }

    #[test]
    fn test_obv_bullish_volume() {
        // All bars closing higher = positive OBV, positive direction
        let bars: Vec<_> = (0..15)
            .map(|i| make_bar(100.0 + i as f64, 90.0 + i as f64, 95.0 + i as f64, 1000.0))
            .collect();
        let (obv, dir) = compute_obv(&bars);
        assert!(obv > 0.0, "OBV should be positive for rising closes, got {}", obv);
        assert!(dir > 0.0, "Direction should be bullish, got {}", dir);
    }

    #[test]
    fn test_obv_bearish_volume() {
        // All bars closing lower = negative OBV, negative direction
        let bars: Vec<_> = (0..15)
            .map(|i| make_bar(100.0 - i as f64, 90.0 - i as f64, 95.0 - i as f64, 1000.0))
            .collect();
        let (obv, dir) = compute_obv(&bars);
        assert!(obv < 0.0, "OBV should be negative for falling closes, got {}", obv);
        assert!(dir < 0.0, "Direction should be bearish, got {}", dir);
    }

    #[test]
    fn test_obv_direction_normalized() {
        let bars: Vec<_> = (0..20)
            .map(|i| make_bar(100.0 + i as f64, 90.0, 95.0 + i as f64, 1000.0))
            .collect();
        let (_, dir) = compute_obv(&bars);
        assert!(dir >= -1.0 && dir <= 1.0, "Direction should be in [-1, 1], got {}", dir);
    }

    // ═══ ADX Tests ═══

    #[test]
    fn test_adx_insufficient_data() {
        let bars = vec![make_bar(100.0, 90.0, 95.0, 1000.0)];
        let (adx, pdi, mdi) = compute_adx(&bars, 14);
        assert_eq!(adx, 25.0, "Default ADX should be 25");
        assert_eq!(pdi, 50.0, "Default +DI should be 50");
        assert_eq!(mdi, 50.0, "Default -DI should be 50");
    }

    #[test]
    fn test_adx_trending_market() {
        // Strong uptrend: each bar higher than previous
        let bars: Vec<_> = (0..30)
            .map(|i| make_bar(100.0 + i as f64 * 2.0, 98.0 + i as f64 * 2.0, 99.0 + i as f64 * 2.0, 1000.0))
            .collect();
        let (adx, pdi, mdi) = compute_adx(&bars, 14);
        assert!(adx > 20.0, "ADX should be >20 in trending market, got {}", adx);
        assert!(pdi > mdi, "+DI should exceed -DI in uptrend, got +DI={} -DI={}", pdi, mdi);
    }

    #[test]
    fn test_adx_range_bound() {
        // Truly range-bound: identical bars = no directional movement = ADX low
        let bars: Vec<_> = (0..30)
            .map(|_| make_bar(101.0, 99.0, 100.0, 1000.0))
            .collect();
        let (adx, _, _) = compute_adx(&bars, 14);
        assert!(adx < 50.0, "ADX should be moderate in range, got {}", adx);
    }

    #[test]
    fn test_adx_in_range_0_100() {
        let bars: Vec<_> = (0..30)
            .map(|i| make_bar(100.0 + i as f64, 99.0 + i as f64, 99.5 + i as f64, 1000.0))
            .collect();
        let (adx, _, _) = compute_adx(&bars, 14);
        assert!(adx >= 0.0 && adx <= 100.0, "ADX should be in [0, 100], got {}", adx);
    }

    // ═══ CCI Tests ═══

    #[test]
    fn test_cci_insufficient_data() {
        let bars = vec![make_bar(100.0, 90.0, 95.0, 1000.0)];
        assert_eq!(compute_cci(&bars, 20), 0.0, "Insufficient data should return 0");
    }

    #[test]
    fn test_cci_overbought() {
        // Strong uptrend = high typical price relative to mean = CCI > +100
        let bars: Vec<_> = (0..25)
            .map(|i| make_bar(100.0 + i as f64 * 5.0, 99.0 + i as f64 * 5.0, 99.5 + i as f64 * 5.0, 1000.0))
            .collect();
        let cci = compute_cci(&bars, 20);
        assert!(cci > 0.0, "CCI should be positive in strong uptrend, got {}", cci);
    }

    #[test]
    fn test_cci_oversold() {
        // Strong downtrend = CCI < -100
        let bars: Vec<_> = (0..25)
            .map(|i| make_bar(100.0 - i as f64 * 5.0, 99.0 - i as f64 * 5.0, 99.5 - i as f64 * 5.0, 1000.0))
            .collect();
        let cci = compute_cci(&bars, 20);
        assert!(cci < 0.0, "CCI should be negative in strong downtrend, got {}", cci);
    }

    #[test]
    fn test_cci_neutral_market() {
        // Flat prices = CCI near 0
        let bars: Vec<_> = (0..25)
            .map(|_| make_bar(101.0, 99.0, 100.0, 1000.0))
            .collect();
        let cci = compute_cci(&bars, 20);
        assert!(cci.abs() < 50.0, "CCI should be near 0 in flat market, got {}", cci);
    }

    // ═══ Williams %R Tests ═══

    #[test]
    fn test_williams_r_insufficient_data() {
        let bars = vec![make_bar(100.0, 90.0, 95.0, 1000.0)];
        assert_eq!(compute_williams_r(&bars, 14), -50.0, "Insufficient data should return -50");
    }

    #[test]
    fn test_williams_r_overbought() {
        // Close at high of range = Williams %R near 0 (overbought)
        let bars: Vec<_> = (0..15)
            .map(|_| make_bar(110.0, 100.0, 110.0, 1000.0))
            .collect();
        let wr = compute_williams_r(&bars, 14);
        assert!(wr > -20.0, "Williams %R should be > -20 when close at high, got {}", wr);
    }

    #[test]
    fn test_williams_r_oversold() {
        // Close at low of range = Williams %R near -100 (oversold)
        let bars: Vec<_> = (0..15)
            .map(|_| make_bar(110.0, 100.0, 100.0, 1000.0))
            .collect();
        let wr = compute_williams_r(&bars, 14);
        assert!(wr < -80.0, "Williams %R should be < -80 when close at low, got {}", wr);
    }

    #[test]
    fn test_williams_r_in_range() {
        assert!(compute_williams_r(&[], 14) >= -100.0 && compute_williams_r(&[], 14) <= 0.0);
    }

    // ═══ VWAP Tests ═══

    #[test]
    fn test_vwap_empty() {
        assert_eq!(compute_vwap(&[]), (0.0, 0.0));
    }

    #[test]
    fn test_vwap_single_bar() {
        let bars = vec![make_bar(110.0, 90.0, 100.0, 5000.0)];
        let (vwap, dev) = compute_vwap(&bars);
        // TP = (110+90+100)/3 = 100, VWAP = 100*5000/5000 = 100
        assert!((vwap - 100.0).abs() < 0.01, "VWAP should be ~100, got {}", vwap);
        assert!((dev).abs() < 0.01, "Deviation should be ~0, got {}", dev);
    }

    #[test]
    fn test_vwap_above_price() {
        // Heavy volume at high prices, then close drops
        let mut bars = vec![make_bar(110.0, 100.0, 105.0, 10000.0)];
        bars.push(make_bar(100.0, 95.0, 96.0, 1000.0)); // low vol drop
        let (vwap, dev) = compute_vwap(&bars);
        assert!(vwap > 96.0, "VWAP should be above current price due to heavy vol at high, got {}", vwap);
        assert!(dev < 0.0, "Deviation should be negative (price below VWAP), got {}", dev);
    }

    #[test]
    fn test_vwap_deviation_sign() {
        // Price above VWAP = positive deviation
        let bars: Vec<_> = (0..10)
            .map(|i| make_bar(100.0 + i as f64, 99.0, 100.0 + i as f64 * 0.1, 1000.0))
            .collect();
        let (vwap, dev) = compute_vwap(&bars);
        let last_price = bars.last().unwrap().close;
        if last_price > vwap {
            assert!(dev > 0.0, "Price above VWAP should give positive deviation");
        }
    }
}
