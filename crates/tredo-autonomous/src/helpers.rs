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
pub fn compute_autonomous_levels(
    symbol: &str,
    current_price: f64,
    pivots: &tredo_core::PivotLevels,
    _patterns: &[tredo_core::CandlestickPattern], // patterns come from MI state; kept for future extension
    regime: crate::types::MarketRegime,
    rsi: f64,
    macd_hist: f64,
    atr_pct: f64,
    rules: &DisciplineRules,
    aggregated_signal: Option<&tredo_core::AggregatedSignal>,
) -> (f64, f64, f64, f64) {  // entry, sl, tp, rr
    let mut direction = tredo_core::TradeDirection::Long;
    let mut entry = current_price;

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
        } else if rsi < 30.0 || (regime == crate::types::MarketRegime::TrendingBull && macd_hist > 0.0) {
            direction = tredo_core::TradeDirection::Long;
        }
    }

    // Entry near current or breakout, biased by aggregated conviction
    let breakout_buffer = 0.001 + (agg_bias.abs() * 0.003);
    entry = if direction == tredo_core::TradeDirection::Long {
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

    let rr = if risk > 0.0 { risk / (tp - entry).abs().max(0.0001) } else { 1.0 };

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

    (entry, sl, final_tp, if risk > 0.0 { risk / (final_tp - entry).abs() } else { 2.0 })
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
    if high == low { return 50.0; }
    ((close - low) / (high - low) * 100.0).clamp(0.0, 100.0)
}

pub fn compute_relative_volume(bars: &[OhlcvBar]) -> f64 {
    if bars.len() < 10 {
        return 1.0;
    }
    let vols: Vec<f64> = bars.iter().map(|b| b.volume).collect();
    let avg: f64 = vols.iter().take(vols.len() - 1).sum::<f64>() / (vols.len() - 1) as f64;
    if avg <= 0.0 { return 1.0; }
    (vols.last().copied().unwrap_or(avg) / avg).clamp(0.3, 3.0)
}

// Simple fib retracements helper (used by meter for levels awareness; agent still decides actual SL/TP)
pub fn compute_fib_levels(high: f64, low: f64) -> (f64, f64) {
    let rng = (high - low).max(0.0001);
    (high - rng * 0.382, high - rng * 0.618)
}
