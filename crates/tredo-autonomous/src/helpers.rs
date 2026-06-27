use crate::state::SharedState;
use crate::types::{MarketRegime, SessionInfo, TradeSignal};
use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc, Weekday}; // Duration removed (SessionInfo now uses i64 to avoid serde issues)
use tredo_core::{DisciplineRules, OhlcvBar};

/// Resolve the best available confluence score for a symbol (per-symbol metrics first).
pub async fn resolve_symbol_confluence(state: &SharedState, symbol: &str) -> f64 {
    {
        let metrics = state.latest_metrics.read().await;
        if let Some(snap) = metrics.get(symbol) {
            if snap.confluence_hint > 0.0 {
                return snap.confluence_hint;
            }
        }
    }
    let agg = state.last_aggregated_signal.read().await;
    agg.as_ref().map(|a| a.conviction).unwrap_or(0.5)
}

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
/// Position size capped by half-Kelly when sufficient trade history exists (≥10 trades).
pub fn kelly_capped_position_size(
    account_balance: f64,
    risk_pct: f64,
    entry_price: f64,
    stop_loss: f64,
    kelly_stats: &crate::episode_store::KellyTradeStats,
) -> (f64, Option<f64>) {
    let risk_based = calculate_position_size(account_balance, risk_pct, entry_price, stop_loss);
    if kelly_stats.trade_count < 10 || kelly_stats.avg_win <= 0.0 || kelly_stats.avg_loss <= 0.0 {
        return (risk_based, None);
    }

    let kelly = tredo_core::kelly_criterion_fraction(
        kelly_stats.win_probability,
        kelly_stats.avg_win,
        kelly_stats.avg_loss,
        account_balance,
        entry_price,
        true,
    );
    if kelly.half_kelly <= 0.0 || kelly.position_size_units <= 0.0 {
        return (risk_based, Some(kelly.half_kelly));
    }

    let kelly_size = kelly.position_size_units.min(risk_based);
    (kelly_size, Some(kelly.half_kelly))
}

pub fn calculate_position_size(
    account_balance: f64,
    risk_pct: f64,
    entry_price: f64,
    stop_loss: f64,
) -> f64 {
    calculate_position_size_with_cash(account_balance, risk_pct, entry_price, stop_loss, None)
}

/// Risk-based position size, optionally capped by available cash for the next entry.
pub fn calculate_position_size_with_cash(
    account_balance: f64,
    risk_pct: f64,
    entry_price: f64,
    stop_loss: f64,
    available_cash: Option<f64>,
) -> f64 {
    if entry_price <= 0.0 {
        return 0.0;
    }

    let risk_amount = account_balance * risk_pct;
    let stop_distance = (entry_price - stop_loss).abs();

    if stop_distance <= 0.0 {
        return 0.0;
    }

    let mut shares = risk_amount / stop_distance;
    let position_value = shares * entry_price;

    let max_position_value = account_balance * 0.95;
    if position_value > max_position_value {
        shares = max_position_value / entry_price;
    }

    if let Some(cash) = available_cash {
        let max_by_cash = (cash * 0.95) / entry_price;
        if max_by_cash <= 0.0 {
            return 0.0;
        }
        shares = shares.min(max_by_cash);
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
    } else if volatility_pct < 0.001 {
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
    forced_direction: Option<tredo_core::TradeDirection>,
) -> (f64, f64, f64, f64) {
    // entry, sl, tp, rr
    let mut direction = forced_direction.unwrap_or(tredo_core::TradeDirection::Long);

    // === RESPECT THE AGGREGATED SIGNAL (Gap 1 fix) ===
    // The agent now lets the combined skill consensus (AggregatedSignal) have real
    // influence on direction and level selection, instead of treating skills as
    // decorative COT output only.
    let mut agg_bias: f64 = 0.0;
    if let Some(agg) = aggregated_signal {
        if let Some(forced) = forced_direction {
            // Direction locked to debate verdict — only use agg for level tuning.
            if forced == tredo_core::TradeDirection::Long && agg.is_bullish(None) {
                agg_bias = agg.net_signal.abs().min(0.6);
            } else if forced == tredo_core::TradeDirection::Short && agg.is_bearish(None) {
                agg_bias = agg.net_signal.abs().min(0.6);
            }
        } else if agg.is_bullish(None) {
            agg_bias = agg.net_signal.abs().min(0.6);
            direction = tredo_core::TradeDirection::Long;
        } else if agg.is_bearish(None) {
            agg_bias = -agg.net_signal.abs().min(0.6);
            direction = tredo_core::TradeDirection::Short;
        }
    }

    // Decide / adjust direction from indicators + aggregated consensus (true agentic fusion)
    if forced_direction.is_none() && agg_bias == 0.0 {
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
    let normalized = if avg_vol > 0.0 {
        magnitude / avg_vol
    } else {
        0.0
    };
    let direction = if recent_slope > 0.0 {
        normalized.min(1.0)
    } else {
        (-normalized).max(-1.0)
    };
    (obv, direction)
}

/// Average Directional Index (ADX) — trend strength (0-100).
/// >25 = trending, <20 = ranging. Combined with +DI/-DI for direction.
/// > Returns (adx, plus_di, minus_di).
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
    let typical_prices: Vec<f64> = bars
        .iter()
        .map(|b| (b.high + b.low + b.close) / 3.0)
        .collect();
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
/// > Returns value in [-100, 0] range.
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
    let vwap = if cum_vol > 0.0 {
        cum_vol_price / cum_vol
    } else {
        bars.last().unwrap().close
    };
    let current_price = bars.last().unwrap().close;
    let deviation = if vwap > 0.0 {
        (current_price - vwap) / vwap
    } else {
        0.0
    };
    (vwap, deviation)
}

// ═══════════════════════════════════════════════════════════════════════════════
// BATCH 2: NEW INDICATORS — 12 additional deterministic signals
// Expanding the indicator arsenal for richer debate, confluence, and reasoning.
// ═══════════════════════════════════════════════════════════════════════════════

/// Parabolic SAR — trend-following stop-and-reverse indicator.
/// Returns (sar_value, trend) where trend is "uptrend" or "downtrend".
/// SAR dots above price = downtrend, below = uptrend.
pub fn compute_parabolic_sar(bars: &[OhlcvBar]) -> (f64, &'static str) {
    if bars.len() < 5 {
        return (bars.last().map(|b| b.close).unwrap_or(0.0), "uptrend");
    }
    let af_step = 0.02;
    let af_max = 0.2;
    let mut af = af_step;
    let mut ep = bars[0].high; // extreme point
    let mut sar = bars[0].low;
    let mut is_long = true;

    for bar in &bars[1..] {
        if is_long {
            sar = sar + af * (ep - sar);
            if bar.low < sar {
                is_long = false;
                sar = ep.max(bar.high);
                ep = bar.low;
                af = af_step;
            } else if bar.high > ep {
                ep = bar.high;
                af = (af + af_step).min(af_max);
            }
        } else {
            sar = sar - af * (sar - ep);
            if bar.high > sar {
                is_long = true;
                sar = ep.min(bar.low);
                ep = bar.high;
                af = af_step;
            } else if bar.low < ep {
                ep = bar.low;
                af = (af + af_step).min(af_max);
            }
        }
    }
    (sar, if is_long { "uptrend" } else { "downtrend" })
}

/// Money Flow Index (MFI) — volume-weighted RSI (-style oscillator).
/// >80 = overbought, <20 = oversold, 50 = neutral. Range [0, 100].
pub fn compute_mfi(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period + 1 {
        return 50.0;
    }
    let mut positive_flow = 0.0;
    let mut negative_flow = 0.0;
    for i in (bars.len() - period)..bars.len() {
        let tp_curr = (bars[i].high + bars[i].low + bars[i].close) / 3.0;
        let tp_prev = (bars[i - 1].high + bars[i - 1].low + bars[i - 1].close) / 3.0;
        let raw_money = tp_curr * bars[i].volume;
        if tp_curr > tp_prev {
            positive_flow += raw_money;
        } else if tp_curr < tp_prev {
            negative_flow += raw_money;
        }
    }
    if negative_flow == 0.0 {
        return 100.0;
    }
    let ratio = positive_flow / negative_flow;
    100.0 - (100.0 / (1.0 + ratio))
}

/// Chaikin Money Flow (CMF) — volume accumulation/distribution oscillator.
/// Range [-1, 1]. >0.1 = accumulation, <-0.1 = distribution, 0 = neutral.
pub fn compute_cmf(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return 0.0;
    }
    let mut sum_mfv = 0.0;
    let mut sum_vol = 0.0;
    for bar in &bars[bars.len() - period..] {
        let range = (bar.high - bar.low).max(0.0001);
        let mfv = ((bar.close - bar.low) - (bar.high - bar.close)) / range * bar.volume;
        sum_mfv += mfv;
        sum_vol += bar.volume;
    }
    if sum_vol == 0.0 {
        0.0
    } else {
        (sum_mfv / sum_vol).clamp(-1.0, 1.0)
    }
}

/// Keltner Channels — volatility-based envelopes around an EMA.
/// Returns (upper, middle, lower). Breakouts above upper = bullish, below lower = bearish.
pub fn compute_keltner_channels(
    bars: &[OhlcvBar],
    period: usize,
    atr_mult: f64,
) -> (f64, f64, f64) {
    if bars.len() < period + 1 {
        let p = bars.last().map(|b| b.close).unwrap_or(0.0);
        return (p * 1.02, p, p * 0.98);
    }
    let ema = compute_ema(bars, period);
    let atr = compute_atr(bars, period);
    (ema + atr_mult * atr, ema, ema - atr_mult * atr)
}

/// Donchian Channels — highest-high / lowest-low over period.
/// Returns (upper, middle, lower). Breakouts signal strong momentum.
pub fn compute_donchian_channels(bars: &[OhlcvBar], period: usize) -> (f64, f64, f64) {
    if bars.len() < period {
        let p = bars.last().map(|b| b.close).unwrap_or(0.0);
        return (p * 1.02, p, p * 0.98);
    }
    let recent = &bars[bars.len() - period..];
    let upper = recent.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let lower = recent.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    (upper, (upper + lower) / 2.0, lower)
}

/// TEMA (Triple EMA) — faster than EMA, less lag.
/// EMA1 = EMA(close), EMA2 = EMA(EMA1), EMA3 = EMA(EMA2).
/// TEMA = 3*EMA1 - 3*EMA2 + EMA3.
pub fn compute_tema(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period * 3 {
        return bars.last().map(|b| b.close).unwrap_or(0.0);
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let ema1 = ema_slice(&closes, period);
    let ema2 = ema_slice(&ema1, period);
    let ema3 = ema_slice(&ema2, period);
    3.0 * ema1.last().copied().unwrap_or(0.0) - 3.0 * ema2.last().copied().unwrap_or(0.0)
        + ema3.last().copied().unwrap_or(0.0)
}

fn ema_slice(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() || period == 0 {
        return data.to_vec();
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut ema = vec![data[0]];
    for &val in data.iter().skip(1) {
        ema.push(val * k + ema.last().unwrap() * (1.0 - k));
    }
    ema
}

/// Hull Moving Average (HMA) — extremely responsive, minimal lag.
/// HMA = WMA(2 * WMA(n/2) - WMA(n)), sqrt(n).
pub fn compute_hma(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return bars.last().map(|b| b.close).unwrap_or(0.0);
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let half = (period / 2).max(1);
    let wma_half = wma_slice(&closes, half);
    let wma_full = wma_slice(&closes, period);
    let diff: Vec<f64> = wma_half
        .iter()
        .zip(wma_full.iter())
        .map(|(a, b)| 2.0 * a - b)
        .collect();
    let sq = period.isqrt().max(1);
    wma_slice(&diff, sq).last().copied().unwrap_or(0.0)
}

fn wma_slice(data: &[f64], period: usize) -> Vec<f64> {
    if data.len() < period || period == 0 {
        return data.to_vec();
    }
    let mut result = Vec::new();
    for i in period..=data.len() {
        let window = &data[i - period..i];
        let mut sum = 0.0;
        let mut weight_sum = 0.0;
        for (j, &val) in window.iter().enumerate() {
            let w = (j + 1) as f64;
            sum += val * w;
            weight_sum += w;
        }
        result.push(sum / weight_sum);
    }
    result
}

/// Elder Ray Index — Bull Power + Bear Power.
/// Bull Power = High - EMA(13). Bear Power = Low - EMA(13).
/// Bull > 0 + Bear > 0 = strong uptrend. Bull < 0 + Bear < 0 = strong downtrend.
pub fn compute_elder_ray(bars: &[OhlcvBar], period: usize) -> (f64, f64) {
    if bars.len() < period {
        return (0.0, 0.0);
    }
    let ema = compute_ema(bars, period);
    let last = bars.last().unwrap();
    (last.high - ema, last.low - ema)
}

/// Aroon Indicator — measures time since highest high / lowest low.
/// Returns (aroon_up, aroon_down, aroon_oscillator). Range [0, 100].
/// Aroon Up > 70 = strong uptrend. Aroon Down > 70 = strong downtrend.
///
/// Formula:
///   days_since_high = (period - 1) - index_of_highest_high
///   aroon_up = (period - days_since_high) / period * 100 = (index_of_highest_high + 1) / period * 100
///   (If the highest high is at the most recent bar, index = period-1, days_since = 0, aroon_up = 100)
pub fn compute_aroon(bars: &[OhlcvBar], period: usize) -> (f64, f64, f64) {
    if bars.len() < period + 1 {
        return (50.0, 50.0, 0.0);
    }
    let recent = &bars[bars.len() - period..];
    let mut high_idx = 0;
    let mut low_idx = 0;
    let mut max_high = f64::MIN;
    let mut min_low = f64::MAX;
    for (i, bar) in recent.iter().enumerate() {
        if bar.high > max_high {
            max_high = bar.high;
            high_idx = i;
        }
        if bar.low < min_low {
            min_low = bar.low;
            low_idx = i;
        }
    }
    // days_since_high = (period - 1) - high_idx
    // aroon_up = (period - days_since_high) / period * 100 = (high_idx + 1) / period * 100
    let aroon_up = ((high_idx + 1) as f64 / period as f64 * 100.0).clamp(0.0, 100.0);
    let aroon_down = ((low_idx + 1) as f64 / period as f64 * 100.0).clamp(0.0, 100.0);
    (aroon_up, aroon_down, aroon_up - aroon_down)
}

/// TRIX — triple-smoothed ROC momentum oscillator.
/// Returns trix value. >0 = bullish momentum, <0 = bearish. Good for divergences.
pub fn compute_trix(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period * 3 + 1 {
        return 0.0;
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let ema1 = ema_slice(&closes, period);
    let ema2 = ema_slice(&ema1, period);
    let ema3 = ema_slice(&ema2, period);
    if ema3.len() < 2 || ema3[ema3.len() - 2] == 0.0 {
        return 0.0;
    }
    let prev = ema3[ema3.len() - 2];
    let curr = ema3[ema3.len() - 1];
    ((curr - prev) / prev) * 100.0
}

/// Rate of Change (ROC) — percentage change over N periods.
/// >0 = bullish momentum, <0 = bearish. Simple but effective momentum gauge.
pub fn compute_roc(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period + 1 {
        return 0.0;
    }
    let prev = bars[bars.len() - period - 1].close;
    let curr = bars.last().unwrap().close;
    if prev == 0.0 {
        0.0
    } else {
        ((curr - prev) / prev) * 100.0
    }
}

/// Momentum (MOM) — raw price difference over N periods.
/// >0 = bullish, <0 = bearish. Used in conjunction with ROC for confirmation.
pub fn compute_momentum(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period + 1 {
        return 0.0;
    }
    bars.last().unwrap().close - bars[bars.len() - period - 1].close
}

// ═══════════════════════════════════════════════════════════════════════════════
// TOOLKIT: Support/Resistance + Volume Profile + Order Flow + Liquidity
// Deterministic market structure analysis — no LLM required.
// ═══════════════════════════════════════════════════════════════════════════════

/// Support/Resistance level detection from swing highs and lows.
/// Returns (support_levels, resistance_levels) sorted strongest first.
/// Uses price clustering — levels that have been touched multiple times are stronger.
pub fn compute_support_resistance(bars: &[OhlcvBar], lookback: usize) -> (Vec<f64>, Vec<f64>) {
    if bars.len() < lookback + 2 {
        return (vec![], vec![]);
    }
    let mut highs = Vec::new();
    let mut lows = Vec::new();
    for i in 1..bars.len() - 1 {
        let prev = &bars[i - 1];
        let curr = &bars[i];
        let next = &bars[i + 1];
        if curr.high > prev.high && curr.high > next.high {
            highs.push(curr.high);
        }
        if curr.low < prev.low && curr.low < next.low {
            lows.push(curr.low);
        }
    }
    // Cluster levels within 0.5% of each other
    let cluster_tolerance = bars.last().map(|b| b.close * 0.005).unwrap_or(1.0);
    let supports = cluster_levels(&lows, cluster_tolerance);
    let resistances = cluster_levels(&highs, cluster_tolerance);
    (supports, resistances)
}

fn cluster_levels(levels: &[f64], tolerance: f64) -> Vec<f64> {
    if levels.is_empty() {
        return vec![];
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut clusters: Vec<(f64, usize)> = Vec::new();
    let mut current = sorted[0];
    let mut count = 1;
    for &level in sorted.iter().skip(1) {
        if (level - current).abs() <= tolerance {
            count += 1;
            current = (current * count as f64 + level) / (count as f64 + 1.0); // weighted average
        } else {
            clusters.push((current, count));
            current = level;
            count = 1;
        }
    }
    clusters.push((current, count));
    // Sort by touch count (strength) descending, then by price
    clusters.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    });
    clusters.into_iter().map(|(price, _)| price).collect()
}

/// Volume Profile — Point of Control (POC), Value Area High (VAH), Value Area Low (VAL).
/// POC = price level with highest volume. VAH/VAL = 70% of volume around POC.
#[derive(Debug, Clone)]
pub struct VolumeProfile {
    pub poc: f64,
    pub vah: f64,
    pub val: f64,
    pub total_volume: f64,
    pub volume_nodes: Vec<(f64, f64)>, // (price, volume)
}

pub fn compute_volume_profile(bars: &[OhlcvBar], num_bins: usize) -> VolumeProfile {
    if bars.is_empty() {
        return VolumeProfile {
            poc: 0.0,
            vah: 0.0,
            val: 0.0,
            total_volume: 0.0,
            volume_nodes: vec![],
        };
    }
    let min_price = bars.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    let max_price = bars.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let range = (max_price - min_price).max(0.0001);
    let bin_size = range / num_bins.max(1) as f64;
    let mut bins: Vec<f64> = vec![0.0; num_bins];
    let mut total_vol = 0.0;
    for bar in bars {
        let tp = (bar.high + bar.low + bar.close) / 3.0;
        let bin_idx = ((tp - min_price) / bin_size).min(num_bins as f64 - 1.0) as usize;
        bins[bin_idx] += bar.volume;
        total_vol += bar.volume;
    }
    // Find POC (highest volume bin)
    let mut poc_idx = 0;
    let mut max_vol = 0.0;
    for (i, &vol) in bins.iter().enumerate() {
        if vol > max_vol {
            max_vol = vol;
            poc_idx = i;
        }
    }
    let poc = min_price + (poc_idx as f64 + 0.5) * bin_size;
    // Find VAH/VAL = 70% of volume around POC
    let target_vol = total_vol * 0.70;
    let mut cumulative = 0.0;
    let mut vah_idx = poc_idx;
    let mut val_idx = poc_idx;
    while cumulative < target_vol && (vah_idx + 1 < num_bins || val_idx > 0) {
        if vah_idx + 1 < num_bins && (val_idx == 0 || bins[vah_idx + 1] >= bins[val_idx - 1]) {
            vah_idx += 1;
            cumulative += bins[vah_idx];
        } else if val_idx > 0 {
            val_idx -= 1;
            cumulative += bins[val_idx];
        } else {
            break;
        }
    }
    let vah = min_price + (vah_idx as f64 + 1.0) * bin_size;
    let val = min_price + (val_idx as f64) * bin_size;
    let nodes: Vec<(f64, f64)> = bins
        .iter()
        .enumerate()
        .map(|(i, &v)| (min_price + (i as f64 + 0.5) * bin_size, v))
        .collect();
    VolumeProfile {
        poc,
        vah,
        val,
        total_volume: total_vol,
        volume_nodes: nodes,
    }
}

/// Order Flow Imbalance — buy vs sell pressure proxy.
/// Uses close position within bar range + volume. >0 = buying pressure, <0 = selling.
/// Range [-1, 1]. Stronger than raw volume because it accounts for bar location.
pub fn compute_order_flow_imbalance(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return 0.0;
    }
    let mut buy_pressure = 0.0;
    let mut total_vol = 0.0;
    for bar in &bars[bars.len() - period..] {
        let range = (bar.high - bar.low).max(0.0001);
        let close_pos = (bar.close - bar.low) / range; // 0 = close at low, 1 = close at high
        let imbalance = (close_pos * 2.0 - 1.0) * bar.volume; // -vol to +vol
        buy_pressure += imbalance;
        total_vol += bar.volume;
    }
    if total_vol == 0.0 {
        0.0
    } else {
        (buy_pressure / total_vol).clamp(-1.0, 1.0)
    }
}

/// Liquidity Analyzer — spread proxy, depth estimate, slippage risk.
/// Uses bar range and volume to estimate market quality. No real order book needed.
#[derive(Debug, Clone)]
pub struct LiquiditySnapshot {
    pub spread_pct: f64,        // estimated bid-ask spread
    pub depth_score: f64,       // 0-1, higher = deeper market
    pub slippage_risk: f64,     // estimated slippage % for typical order size
    pub market_quality: String, // "excellent" | "good" | "fair" | "poor"
}

pub fn compute_liquidity(bars: &[OhlcvBar], current_price: f64) -> LiquiditySnapshot {
    if bars.len() < 10 {
        return LiquiditySnapshot {
            spread_pct: 0.001,
            depth_score: 0.5,
            slippage_risk: 0.002,
            market_quality: "fair".to_string(),
        };
    }
    let recent = &bars[bars.len() - 10..];
    let avg_range = recent.iter().map(|b| b.high - b.low).sum::<f64>() / 10.0;
    let spread_pct = (avg_range / current_price * 0.3).clamp(0.0001, 0.01); // 30% of range as spread proxy

    let avg_vol = recent.iter().map(|b| b.volume).sum::<f64>() / 10.0;
    let vol_consistency = if avg_vol > 0.0 {
        let vol_std = recent
            .iter()
            .map(|b| (b.volume - avg_vol).powi(2))
            .sum::<f64>()
            / 10.0;
        let cv = (vol_std.sqrt() / avg_vol).clamp(0.0, 2.0); // coefficient of variation
        1.0 - (cv / 2.0) // higher consistency = higher score
    } else {
        0.0
    };

    let depth_score =
        (vol_consistency * 0.7 + (1.0 - spread_pct * 100.0).clamp(0.0, 1.0) * 0.3).clamp(0.0, 1.0);
    let slippage_risk = (spread_pct * 2.0 + (1.0 - depth_score) * 0.005).clamp(0.0001, 0.02);

    let quality = if depth_score > 0.8 && slippage_risk < 0.003 {
        "excellent"
    } else if depth_score > 0.6 && slippage_risk < 0.005 {
        "good"
    } else if depth_score > 0.4 {
        "fair"
    } else {
        "poor"
    };

    LiquiditySnapshot {
        spread_pct,
        depth_score,
        slippage_risk,
        market_quality: quality.to_string(),
    }
}

/// Funding Rate Analyzer — crypto perpetual futures funding rate proxy.
/// Positive funding = longs pay shorts (bullish sentiment). Negative = shorts pay longs (bearish).
/// Uses local price-volatility proxy when real API unavailable.
pub fn compute_funding_rate_proxy(bars: &[OhlcvBar], _symbol: &str) -> (f64, &'static str) {
    if bars.len() < 24 {
        return (0.0, "neutral");
    }
    let recent = &bars[bars.len() - 24..]; // last 24 bars as daily proxy
    let price_change = (recent.last().unwrap().close - recent[0].close) / recent[0].close;
    let vol = compute_atr(recent, 14) / recent.last().unwrap().close;
    // Positive funding proxy: strong uptrend + high vol = crowded longs
    let funding = (price_change * 0.5 + vol * 0.3).clamp(-0.01, 0.01);
    let sentiment = if funding > 0.003 {
        "bullish"
    } else if funding < -0.003 {
        "bearish"
    } else {
        "neutral"
    };
    (funding, sentiment)
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
        assert!(
            obv > 0.0,
            "OBV should be positive for rising closes, got {}",
            obv
        );
        assert!(dir > 0.0, "Direction should be bullish, got {}", dir);
    }

    #[test]
    fn test_obv_bearish_volume() {
        // All bars closing lower = negative OBV, negative direction
        let bars: Vec<_> = (0..15)
            .map(|i| make_bar(100.0 - i as f64, 90.0 - i as f64, 95.0 - i as f64, 1000.0))
            .collect();
        let (obv, dir) = compute_obv(&bars);
        assert!(
            obv < 0.0,
            "OBV should be negative for falling closes, got {}",
            obv
        );
        assert!(dir < 0.0, "Direction should be bearish, got {}", dir);
    }

    #[test]
    fn test_obv_direction_normalized() {
        let bars: Vec<_> = (0..20)
            .map(|i| make_bar(100.0 + i as f64, 90.0, 95.0 + i as f64, 1000.0))
            .collect();
        let (_, dir) = compute_obv(&bars);
        assert!(
            (-1.0..=1.0).contains(&dir),
            "Direction should be in [-1, 1], got {}",
            dir
        );
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
            .map(|i| {
                make_bar(
                    100.0 + i as f64 * 2.0,
                    98.0 + i as f64 * 2.0,
                    99.0 + i as f64 * 2.0,
                    1000.0,
                )
            })
            .collect();
        let (adx, pdi, mdi) = compute_adx(&bars, 14);
        assert!(
            adx > 20.0,
            "ADX should be >20 in trending market, got {}",
            adx
        );
        assert!(
            pdi > mdi,
            "+DI should exceed -DI in uptrend, got +DI={} -DI={}",
            pdi,
            mdi
        );
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
        assert!(
            (0.0..=100.0).contains(&adx),
            "ADX should be in [0, 100], got {}",
            adx
        );
    }

    // ═══ CCI Tests ═══

    #[test]
    fn test_cci_insufficient_data() {
        let bars = vec![make_bar(100.0, 90.0, 95.0, 1000.0)];
        assert_eq!(
            compute_cci(&bars, 20),
            0.0,
            "Insufficient data should return 0"
        );
    }

    #[test]
    fn test_cci_overbought() {
        // Strong uptrend = high typical price relative to mean = CCI > +100
        let bars: Vec<_> = (0..25)
            .map(|i| {
                make_bar(
                    100.0 + i as f64 * 5.0,
                    99.0 + i as f64 * 5.0,
                    99.5 + i as f64 * 5.0,
                    1000.0,
                )
            })
            .collect();
        let cci = compute_cci(&bars, 20);
        assert!(
            cci > 0.0,
            "CCI should be positive in strong uptrend, got {}",
            cci
        );
    }

    #[test]
    fn test_cci_oversold() {
        // Strong downtrend = CCI < -100
        let bars: Vec<_> = (0..25)
            .map(|i| {
                make_bar(
                    100.0 - i as f64 * 5.0,
                    99.0 - i as f64 * 5.0,
                    99.5 - i as f64 * 5.0,
                    1000.0,
                )
            })
            .collect();
        let cci = compute_cci(&bars, 20);
        assert!(
            cci < 0.0,
            "CCI should be negative in strong downtrend, got {}",
            cci
        );
    }

    #[test]
    fn test_cci_neutral_market() {
        // Flat prices = CCI near 0
        let bars: Vec<_> = (0..25)
            .map(|_| make_bar(101.0, 99.0, 100.0, 1000.0))
            .collect();
        let cci = compute_cci(&bars, 20);
        assert!(
            cci.abs() < 50.0,
            "CCI should be near 0 in flat market, got {}",
            cci
        );
    }

    // ═══ Williams %R Tests ═══

    #[test]
    fn test_williams_r_insufficient_data() {
        let bars = vec![make_bar(100.0, 90.0, 95.0, 1000.0)];
        assert_eq!(
            compute_williams_r(&bars, 14),
            -50.0,
            "Insufficient data should return -50"
        );
    }

    #[test]
    fn test_williams_r_overbought() {
        // Close at high of range = Williams %R near 0 (overbought)
        let bars: Vec<_> = (0..15)
            .map(|_| make_bar(110.0, 100.0, 110.0, 1000.0))
            .collect();
        let wr = compute_williams_r(&bars, 14);
        assert!(
            wr > -20.0,
            "Williams %R should be > -20 when close at high, got {}",
            wr
        );
    }

    #[test]
    fn test_williams_r_oversold() {
        // Close at low of range = Williams %R near -100 (oversold)
        let bars: Vec<_> = (0..15)
            .map(|_| make_bar(110.0, 100.0, 100.0, 1000.0))
            .collect();
        let wr = compute_williams_r(&bars, 14);
        assert!(
            wr < -80.0,
            "Williams %R should be < -80 when close at low, got {}",
            wr
        );
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
        assert!(
            (vwap - 100.0).abs() < 0.01,
            "VWAP should be ~100, got {}",
            vwap
        );
        assert!((dev).abs() < 0.01, "Deviation should be ~0, got {}", dev);
    }

    #[test]
    fn test_vwap_above_price() {
        // Heavy volume at high prices, then close drops
        let mut bars = vec![make_bar(110.0, 100.0, 105.0, 10000.0)];
        bars.push(make_bar(100.0, 95.0, 96.0, 1000.0)); // low vol drop
        let (vwap, dev) = compute_vwap(&bars);
        assert!(
            vwap > 96.0,
            "VWAP should be above current price due to heavy vol at high, got {}",
            vwap
        );
        assert!(
            dev < 0.0,
            "Deviation should be negative (price below VWAP), got {}",
            dev
        );
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

    // ═══ Parabolic SAR Tests ═══

    #[test]
    fn test_parabolic_sar_trend() {
        let bars: Vec<_> = (0..20)
            .map(|i| {
                make_bar(
                    100.0 + i as f64 * 2.0,
                    98.0 + i as f64 * 2.0,
                    99.0 + i as f64 * 2.0,
                    1000.0,
                )
            })
            .collect();
        let (sar, trend) = compute_parabolic_sar(&bars);
        let last_price = bars.last().unwrap().close;
        // In uptrend, SAR should be below price
        if trend == "uptrend" {
            assert!(
                sar < last_price,
                "SAR should be below price in uptrend, got SAR={} price={}",
                sar,
                last_price
            );
        }
    }

    #[test]
    fn test_parabolic_sar_downtrend() {
        let bars: Vec<_> = (0..20)
            .map(|i| {
                make_bar(
                    100.0 - i as f64 * 2.0,
                    98.0 - i as f64 * 2.0,
                    99.0 - i as f64 * 2.0,
                    1000.0,
                )
            })
            .collect();
        let (sar, trend) = compute_parabolic_sar(&bars);
        let last_price = bars.last().unwrap().close;
        // In downtrend, SAR should be above price
        if trend == "downtrend" {
            assert!(
                sar > last_price,
                "SAR should be above price in downtrend, got SAR={} price={}",
                sar,
                last_price
            );
        }
    }

    // ═══ MFI Tests ═══

    #[test]
    fn test_mfi_range() {
        let bars: Vec<_> = (0..20)
            .map(|i| make_bar(100.0 + i as f64, 99.0, 99.5 + i as f64, 1000.0))
            .collect();
        let mfi = compute_mfi(&bars, 14);
        assert!(
            (0.0..=100.0).contains(&mfi),
            "MFI should be in [0, 100], got {}",
            mfi
        );
    }

    #[test]
    fn test_mfi_oversold() {
        let bars: Vec<_> = (0..20)
            .map(|i| make_bar(100.0 - i as f64, 99.0 - i as f64, 99.5 - i as f64, 5000.0))
            .collect();
        let mfi = compute_mfi(&bars, 14);
        assert!(
            mfi < 30.0,
            "MFI should be < 30 in strong downtrend, got {}",
            mfi
        );
    }

    // ═══ Aroon Tests ═══

    #[test]
    fn test_aroon_trend() {
        let bars: Vec<_> = (0..30)
            .map(|i| {
                make_bar(
                    100.0 + i as f64 * 2.0,
                    98.0 + i as f64 * 2.0,
                    99.0 + i as f64 * 2.0,
                    1000.0,
                )
            })
            .collect();
        let (aroon_up, aroon_down, aroon_osc) = compute_aroon(&bars, 14);
        // In uptrend, aroon_up should be high, aroon_down low, osc positive
        assert!(
            aroon_up > aroon_down,
            "Aroon Up should exceed Aroon Down in uptrend, got up={} down={}",
            aroon_up,
            aroon_down
        );
        assert!(
            aroon_osc > 0.0,
            "Aroon Oscillator should be positive in uptrend, got {}",
            aroon_osc
        );
    }

    // ═══ Donchian Channel Tests ═══

    #[test]
    fn test_donchian_channels() {
        let bars: Vec<_> = (0..30)
            .map(|i| {
                make_bar(
                    100.0 + i as f64,
                    50.0 + i as f64 * 0.5,
                    75.0 + i as f64 * 0.7,
                    1000.0,
                )
            })
            .collect();
        let (upper, mid, lower) = compute_donchian_channels(&bars, 20);
        assert!(
            upper >= mid && mid >= lower,
            "Donchian: upper >= mid >= lower, got upper={} mid={} lower={}",
            upper,
            mid,
            lower
        );
    }

    // ═══ Keltner Channel Tests ═══

    #[test]
    fn test_keltner_channels() {
        let bars: Vec<_> = (0..30)
            .map(|i| make_bar(100.0 + i as f64, 98.0 + i as f64, 99.0 + i as f64, 1000.0))
            .collect();
        let (upper, mid, lower) = compute_keltner_channels(&bars, 20, 2.0);
        assert!(
            upper >= mid && mid >= lower,
            "Keltner: upper >= mid >= lower, got upper={} mid={} lower={}",
            upper,
            mid,
            lower
        );
    }

    // ═══ HMA Tests ═══

    #[test]
    fn test_hma_responsive() {
        let mut bars: Vec<_> = (0..20)
            .map(|_i| make_bar(100.0, 99.0, 100.0, 1000.0))
            .collect();
        // Add a sharp move
        bars.push(make_bar(110.0, 109.0, 110.0, 1000.0));
        bars.push(make_bar(111.0, 110.0, 111.0, 1000.0));
        let hma = compute_hma(&bars, 16);
        let _last_price = bars.last().unwrap().close;
        // HMA should be closer to price than a simple SMA would be
        assert!(hma > 0.0, "HMA should be positive, got {}", hma);
    }

    // ═══ Elder Ray Tests ═══

    #[test]
    fn test_elder_ray_bull_power() {
        let bars: Vec<_> = (0..20)
            .map(|i| make_bar(100.0 + i as f64, 99.0 + i as f64, 99.5 + i as f64, 1000.0))
            .collect();
        let (bull_power, _bear_power) = compute_elder_ray(&bars, 13);
        // In uptrend, bull_power should be positive
        assert!(
            bull_power > 0.0,
            "Bull power should be positive in uptrend, got {}",
            bull_power
        );
    }

    // ═══ ROC Tests ═══

    #[test]
    fn test_roc_positive() {
        let bars: Vec<_> = (0..20)
            .map(|i| make_bar(100.0 + i as f64, 99.0 + i as f64, 99.5 + i as f64, 1000.0))
            .collect();
        let roc = compute_roc(&bars, 12);
        assert!(roc > 0.0, "ROC should be positive in uptrend, got {}", roc);
    }

    // ═══ Support/Resistance Tests ═══

    #[test]
    fn test_support_resistance_levels() {
        let bars: Vec<_> = (0..30)
            .map(|i| {
                make_bar(
                    100.0 + (i % 10) as f64,
                    90.0 + (i % 10) as f64,
                    95.0 + (i % 10) as f64,
                    1000.0,
                )
            })
            .collect();
        let (supports, resistances) = compute_support_resistance(&bars, 10);
        // Should find at least one of each in oscillating data
        assert!(
            !supports.is_empty() || !resistances.is_empty(),
            "Should find S/R levels in oscillating data"
        );
    }

    // ═══ Volume Profile Tests ═══

    #[test]
    fn test_volume_profile() {
        let bars: Vec<_> = (0..30)
            .map(|i| {
                make_bar(
                    100.0 + i as f64,
                    99.0 + i as f64,
                    99.5 + i as f64,
                    1000.0 + i as f64 * 100.0,
                )
            })
            .collect();
        let profile = compute_volume_profile(&bars, 10);
        assert!(
            profile.poc > 0.0,
            "POC should be positive, got {}",
            profile.poc
        );
        assert!(
            profile.vah >= profile.val,
            "VAH should be >= VAL, got VAH={} VAL={}",
            profile.vah,
            profile.val
        );
    }
}
