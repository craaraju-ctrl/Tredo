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

/// Autonomous level calculator: agent decides entry, SL, TP from context + indicators
/// No external price points provided.
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
) -> (f64, f64, f64, f64) {  // entry, sl, tp, rr
    let mut direction = tredo_core::TradeDirection::Long;
    let mut entry = current_price;

    // Decide direction from indicators (agentic)
    if rsi > 70.0 || (regime == crate::types::MarketRegime::TrendingBear && macd_hist < 0.0) {
        direction = tredo_core::TradeDirection::Short;
    } else if rsi < 30.0 || (regime == crate::types::MarketRegime::TrendingBull && macd_hist > 0.0) {
        direction = tredo_core::TradeDirection::Long;
    }

    // Entry near current or breakout
    entry = if direction == tredo_core::TradeDirection::Long {
        current_price.max(pivots.pivot * 1.001)  // slight breakout
    } else {
        current_price.min(pivots.pivot * 0.999)
    };

    // SL using ATR + pivot support (agent identifies protection level)
    let atr = current_price * atr_pct.max(0.01);
    let sl = if direction == tredo_core::TradeDirection::Long {
        (current_price - atr * 1.5).min(pivots.s1)
    } else {
        (current_price + atr * 1.5).max(pivots.r1)
    };

    // TP: 2:1 or 3:1 RR or next pivot target (agentic target based on structure)
    let risk = (entry - sl).abs();
    let tp = if direction == tredo_core::TradeDirection::Long {
        entry + risk * 2.0
    } else {
        entry - risk * 2.0
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
