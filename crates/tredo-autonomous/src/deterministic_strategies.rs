// ═══════════════════════════════════════════════════════════════════════════════
// Deterministic Strategies — LLM-Optional Core Trading Engine
//
// These strategies run WITHOUT LLM or Kronos forecast. They are pure, deterministic
// rule-based trading systems that use only technical indicators and the SuperIntelligence
// conviction layer. The LLM is used only as a cross-check opinion, NOT as the primary
// decision maker.
//
// Strategies:
//   1. MeanReversion — RSI + Bollinger Band mean reversion (range-bound markets)
//   2. TrendContinuation — ADX + MACD trend following (trending markets)
//   3. VolatilityBreakout — ATR + Donchian breakout (volatile/breakout markets)
//   4. SupportResistanceBounce — S/R level bounce with volume confirmation
//
// Each strategy produces a TradeSignal with conviction, direction, and levels.
// The SuperIntelligence layer selects the best strategy (or HOLD) based on regime.
// ═══════════════════════════════════════════════════════════════════════════════

use crate::helpers;
use crate::types::{MarketRegime, TradeSignal};
use chrono::Utc;
use tredo_core::{OhlcvBar, TradeDirection};

/// Base risk per trade (1% of equity by default)
const BASE_RISK_PCT: f64 = 0.01;

/// Result from a deterministic strategy
#[derive(Debug, Clone)]
pub struct StrategyResult {
    pub strategy_name: String,
    pub direction: TradeDirection,
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub confidence: f64,
    pub reason: String,
    /// Which regimes this strategy is suitable for
    pub suitable_regimes: Vec<MarketRegime>,
    pub rsi: f64,
    pub atr_pct: f64,
}

/// Mean Reversion Strategy — RSI + Bollinger Bands
///
/// Best in: Ranging, LowLiquidity markets
/// Signals:
///   - BUY: RSI < 30 (oversold) + price near lower Bollinger Band
///   - SELL: RSI > 70 (overbought) + price near upper Bollinger Band
///   - HOLD: Otherwise
pub fn mean_reversion_strategy(bars: &[OhlcvBar], current_price: f64) -> Option<StrategyResult> {
    if bars.len() < 25 {
        return None;
    }

    let rsi = helpers::compute_rsi(bars, 14);
    let (bb_upper, _bb_mid, bb_lower) = helpers::compute_bollinger_bands(bars, 20, 2.0);
    let atr = helpers::compute_atr(bars, 14);

    let band_width = (bb_upper - bb_lower).max(0.0001);
    let price_position = (current_price - bb_lower) / band_width; // 0 = lower, 1 = upper
    let atr_pct = atr / current_price;

    // BUY signal: oversold + near lower band
    if rsi < 35.0 && price_position < 0.25 {
        let stop_loss = current_price * (1.0 - atr_pct * 1.5).max(0.9);
        let take_profit = current_price * (1.0 + atr_pct * 2.0).min(1.1);
        let confidence =
            ((35.0 - rsi) / 35.0 * 0.6 + (1.0 - price_position) * 0.4).clamp(0.0, 0.95);

        return Some(StrategyResult {
            strategy_name: "MeanReversion".to_string(),
            direction: TradeDirection::Long,
            entry_price: current_price,
            stop_loss,
            take_profit,
            confidence,
            reason: format!(
                "Mean reversion BUY: RSI={:.1} (oversold), price at {:.0}% of BB range",
                rsi,
                price_position * 100.0
            ),
            suitable_regimes: vec![MarketRegime::Ranging, MarketRegime::LowLiquidity],
            rsi,
            atr_pct,
        });
    }

    // SELL signal: overbought + near upper band
    if rsi > 65.0 && price_position > 0.75 {
        let stop_loss = current_price * (1.0 + atr_pct * 1.5).min(1.1);
        let take_profit = current_price * (1.0 - atr_pct * 2.0).max(0.9);
        let confidence = ((rsi - 65.0) / 35.0 * 0.6 + price_position * 0.4).clamp(0.0, 0.95);

        return Some(StrategyResult {
            strategy_name: "MeanReversion".to_string(),
            direction: TradeDirection::Short,
            entry_price: current_price,
            stop_loss,
            take_profit,
            confidence,
            reason: format!(
                "Mean reversion SELL: RSI={:.1} (overbought), price at {:.0}% of BB range",
                rsi,
                price_position * 100.0
            ),
            suitable_regimes: vec![MarketRegime::Ranging, MarketRegime::LowLiquidity],
            rsi,
            atr_pct,
        });
    }

    None
}

/// Trend Continuation Strategy — ADX + MACD
///
/// Best in: TrendingBull, TrendingBear markets
/// Signals:
///   - BUY: ADX > 25 + +DI > -DI + MACD > Signal + price > SMA(50)
///   - SELL: ADX > 25 + -DI > +DI + MACD < Signal + price < SMA(50)
///   - HOLD: Otherwise
pub fn trend_continuation_strategy(
    bars: &[OhlcvBar],
    current_price: f64,
) -> Option<StrategyResult> {
    if bars.len() < 55 {
        return None;
    }

    let (adx, plus_di, minus_di) = helpers::compute_adx(bars, 14);
    let (_, _, macd_hist) = helpers::compute_macd(bars);
    let sma_50 = compute_sma(bars, 50);
    let atr = helpers::compute_atr(bars, 14);
    let atr_pct = atr / current_price;
    let rsi = helpers::compute_rsi(bars, 14);

    // BUY: Strong uptrend
    if adx > 25.0 && plus_di > minus_di && macd_hist > 0.0 && current_price > sma_50 {
        let confidence =
            ((adx - 25.0) / 50.0 * 0.3 + (plus_di - minus_di) / 100.0 * 0.3 + 0.4).clamp(0.0, 0.95);
        let stop_loss = current_price * (1.0 - atr_pct * 2.0).max(0.88);
        let take_profit = current_price * (1.0 + atr_pct * 3.0).min(1.15);

        return Some(StrategyResult {
            strategy_name: "TrendContinuation".to_string(),
            direction: TradeDirection::Long,
            entry_price: current_price,
            stop_loss,
            take_profit,
            confidence,
            reason: format!(
                "Trend continuation BUY: ADX={:.1}, +DI={:.1} > -DI={:.1}, MACD+={:.4}, price>MA50",
                adx, plus_di, minus_di, macd_hist
            ),
            suitable_regimes: vec![MarketRegime::TrendingBull],
            rsi,
            atr_pct,
        });
    }

    // SELL: Strong downtrend
    if adx > 25.0 && minus_di > plus_di && macd_hist < 0.0 && current_price < sma_50 {
        let confidence =
            ((adx - 25.0) / 50.0 * 0.3 + (minus_di - plus_di) / 100.0 * 0.3 + 0.4).clamp(0.0, 0.95);
        let stop_loss = current_price * (1.0 + atr_pct * 2.0).min(1.12);
        let take_profit = current_price * (1.0 - atr_pct * 3.0).max(0.85);

        return Some(StrategyResult {
            strategy_name: "TrendContinuation".to_string(),
            direction: TradeDirection::Short,
            entry_price: current_price,
            stop_loss,
            take_profit,
            confidence,
            reason: format!("Trend continuation SELL: ADX={:.1}, -DI={:.1} > +DI={:.1}, MACD-={:.4}, price<MA50", adx, minus_di, plus_di, macd_hist),
            suitable_regimes: vec![MarketRegime::TrendingBear],
            rsi,
            atr_pct,
        });
    }

    None
}

/// Volatility Breakout Strategy — ATR + Donchian Channels
///
/// Best in: Volatile, TrendingBull markets
/// Signals:
///   - BUY: Price breaks above Donchian upper channel + volume confirmation
///   - SELL: Price breaks below Donchian lower channel + volume confirmation
///   - HOLD: Otherwise
pub fn volatility_breakout_strategy(
    bars: &[OhlcvBar],
    current_price: f64,
) -> Option<StrategyResult> {
    if bars.len() < 25 {
        return None;
    }

    let (donchian_upper, _donchian_mid, donchian_lower) =
        helpers::compute_donchian_channels(bars, 20);
    let atr = helpers::compute_atr(bars, 14);
    let atr_pct = atr / current_price;
    let rel_vol = helpers::compute_relative_volume(bars);
    let rsi = helpers::compute_rsi(bars, 14);

    // BUY: Break above upper Donchian with volume
    if current_price > donchian_upper * 1.002 && rel_vol > 1.2 {
        let confidence =
            ((current_price / donchian_upper - 1.0) * 20.0 * 0.3 + (rel_vol - 1.0) * 0.3 + 0.4)
                .clamp(0.0, 0.95);
        let stop_loss = donchian_mid(bars, 20) * 0.98;
        let take_profit = current_price * (1.0 + atr_pct * 3.0).min(1.2);

        return Some(StrategyResult {
            strategy_name: "VolatilityBreakout".to_string(),
            direction: TradeDirection::Long,
            entry_price: current_price,
            stop_loss,
            take_profit,
            confidence,
            reason: format!(
                "Volatility breakout BUY: price>{:.2} above Donchian upper, rel_vol={:.1}x",
                current_price / donchian_upper,
                rel_vol
            ),
            suitable_regimes: vec![MarketRegime::Volatile, MarketRegime::TrendingBull],
            rsi,
            atr_pct,
        });
    }

    // SELL: Break below lower Donchian with volume
    if current_price < donchian_lower * 0.998 && rel_vol > 1.2 {
        let confidence =
            ((1.0 - current_price / donchian_lower) * 20.0 * 0.3 + (rel_vol - 1.0) * 0.3 + 0.4)
                .clamp(0.0, 0.95);
        let stop_loss = donchian_mid(bars, 20) * 1.02;
        let take_profit = current_price * (1.0 - atr_pct * 3.0).max(0.8);

        return Some(StrategyResult {
            strategy_name: "VolatilityBreakout".to_string(),
            direction: TradeDirection::Short,
            entry_price: current_price,
            stop_loss,
            take_profit,
            confidence,
            reason: format!(
                "Volatility breakout SELL: price<{:.2} below Donchian lower, rel_vol={:.1}x",
                current_price / donchian_lower,
                rel_vol
            ),
            suitable_regimes: vec![MarketRegime::Volatile],
            rsi,
            atr_pct,
        });
    }

    None
}

/// Support/Resistance Bounce Strategy — S/R levels + Volume Profile
///
/// Best in: Ranging, TrendingBull markets
/// Signals:
///   - BUY: Price near support level + bullish candlestick pattern + volume confirmation
///   - SELL: Price near resistance level + bearish candlestick pattern + volume confirmation
pub fn support_resistance_bounce_strategy(
    bars: &[OhlcvBar],
    current_price: f64,
    supports: &[f64],
    resistances: &[f64],
) -> Option<StrategyResult> {
    if bars.len() < 15 || (supports.is_empty() && resistances.is_empty()) {
        return None;
    }

    let atr = helpers::compute_atr(bars, 14);
    let atr_pct = atr / current_price;
    let rsi = helpers::compute_rsi(bars, 14);
    let rel_vol = helpers::compute_relative_volume(bars);

    // Check nearest support level for bounce BUY
    for support in supports.iter().take(3) {
        let distance = ((current_price - support) / current_price).abs();
        if distance < atr_pct * 0.5 && rsi < 50.0 && rel_vol > 0.8 {
            let strength = (1.0 - distance / (atr_pct * 0.5)) * 0.5 + (0.5 - (rsi / 100.0)) * 0.5;
            let confidence = strength.clamp(0.0, 0.95);
            let stop_loss = support * (1.0 - atr_pct).max(0.92);
            let take_profit = current_price * (1.0 + atr_pct * 2.5).min(1.12);

            return Some(StrategyResult {
                strategy_name: "SupportResistanceBounce".to_string(),
                direction: TradeDirection::Long,
                entry_price: current_price,
                stop_loss,
                take_profit,
                confidence,
                reason: format!(
                    "S/R bounce BUY: near support at {:.2} (distance={:.2}%), RSI={:.1}",
                    support,
                    distance * 100.0,
                    rsi
                ),
                suitable_regimes: vec![MarketRegime::Ranging, MarketRegime::TrendingBull],
                rsi,
                atr_pct,
            });
        }
    }

    // Check nearest resistance level for bounce SELL
    for resistance in resistances.iter().take(3) {
        let distance = ((resistance - current_price) / current_price).abs();
        if distance < atr_pct * 0.5 && rsi > 50.0 && rel_vol > 0.8 {
            let strength = (1.0 - distance / (atr_pct * 0.5)) * 0.5 + ((rsi / 100.0) - 0.5) * 0.5;
            let confidence = strength.clamp(0.0, 0.95);
            let stop_loss = resistance * (1.0 + atr_pct).min(1.08);
            let take_profit = current_price * (1.0 - atr_pct * 2.5).max(0.88);

            return Some(StrategyResult {
                strategy_name: "SupportResistanceBounce".to_string(),
                direction: TradeDirection::Short,
                entry_price: current_price,
                stop_loss,
                take_profit,
                confidence,
                reason: format!(
                    "S/R bounce SELL: near resistance at {:.2} (distance={:.2}%), RSI={:.1}",
                    resistance,
                    distance * 100.0,
                    rsi
                ),
                suitable_regimes: vec![MarketRegime::Ranging, MarketRegime::TrendingBear],
                rsi,
                atr_pct,
            });
        }
    }

    None
}

/// Master strategy selector — runs all strategies and picks the best match for current regime.
///
/// Returns the best strategy result based on regime suitability + confidence.
/// If no strategy produces a good signal, returns None (HOLD).
pub fn select_best_strategy(
    bars: &[OhlcvBar],
    current_price: f64,
    regime: &MarketRegime,
    supports: &[f64],
    resistances: &[f64],
) -> Option<StrategyResult> {
    let strategies = [
        mean_reversion_strategy(bars, current_price),
        trend_continuation_strategy(bars, current_price),
        volatility_breakout_strategy(bars, current_price),
        support_resistance_bounce_strategy(bars, current_price, supports, resistances),
    ];

    // Score each strategy by regime suitability + confidence
    let mut best: Option<StrategyResult> = None;
    let mut best_score = 0.0;

    for strategy in strategies.iter().flatten() {
        let regime_suitability = if strategy.suitable_regimes.contains(regime) {
            1.0
        } else {
            // Not ideal but might still work — penalize
            0.5
        };
        let score = strategy.confidence * 0.6 + regime_suitability * 0.4;

        if score > best_score && score > 0.35 {
            best_score = score;
            best = Some(strategy.clone());
        }
    }

    best
}

/// Convert a StrategyResult to a TradeSignal.
pub fn strategy_result_to_signal(result: &StrategyResult, equity: f64) -> TradeSignal {
    let risk_amount = equity * BASE_RISK_PCT;
    let raw_size = risk_amount / (result.entry_price - result.stop_loss).abs().max(0.01);
    // Cap position size at 100% of equity to prevent extreme sizing when SL is very tight
    let max_size = equity / result.entry_price.max(0.01);
    let position_size = raw_size.min(max_size);
    let rr_ratio = (result.take_profit - result.entry_price).abs()
        / (result.stop_loss - result.entry_price).abs().max(0.001);

    TradeSignal {
        symbol: String::new(), // Filled in by caller
        direction: result.direction,
        entry_price: result.entry_price,
        stop_loss: result.stop_loss,
        take_profit: result.take_profit,
        position_size,
        confidence_score: result.confidence,
        confluence_score: result.confidence,
        risk_reward_ratio: rr_ratio,
        reasoning: format!("{} | {}", result.strategy_name, result.reason),
        timestamp: Utc::now(),
        session_valid: true,
        risk_check_passed: true,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Internal helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn compute_sma(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return bars.last().map(|b| b.close).unwrap_or(0.0);
    }
    let sum: f64 = bars[bars.len() - period..].iter().map(|b| b.close).sum();
    sum / period as f64
}

fn donchian_mid(bars: &[OhlcvBar], period: usize) -> f64 {
    if bars.len() < period {
        return bars.last().map(|b| b.close).unwrap_or(0.0);
    }
    let recent = &bars[bars.len() - period..];
    let upper = recent.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let lower = recent.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    (upper + lower) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bars(prices: &[f64]) -> Vec<OhlcvBar> {
        prices
            .iter()
            .enumerate()
            .map(|(i, &p)| OhlcvBar {
                timestamp: format!("2026-01-{:02}T00:00:00+00:00", i + 1),
                open: p * 0.998,
                high: p * 1.01,
                low: p * 0.99,
                close: p,
                volume: 1000.0,
            })
            .collect()
    }

    #[allow(dead_code)]
    fn make_bars_with_volume(prices: &[f64], volumes: &[f64]) -> Vec<OhlcvBar> {
        prices
            .iter()
            .enumerate()
            .map(|(i, &p)| OhlcvBar {
                timestamp: format!("2026-01-{:02}T00:00:00+00:00", i + 1),
                open: p * 0.998,
                high: p * 1.01,
                low: p * 0.99,
                close: p,
                volume: if i < volumes.len() {
                    volumes[i]
                } else {
                    1000.0
                },
            })
            .collect()
    }

    #[test]
    fn test_mean_reversion_oversold() {
        // Create a series that drops sharply (oversold condition)
        let prices: Vec<f64> = (0..30).map(|i| 100.0 - i as f64 * 2.0).collect();
        let bars = make_bars(&prices);
        let result = mean_reversion_strategy(&bars, *prices.last().unwrap());

        // RSI should be < 35 (oversold)
        assert!(
            result.is_some(),
            "Mean reversion should detect oversold condition"
        );
        if let Some(ref r) = result {
            assert_eq!(r.direction, TradeDirection::Long);
            assert!(r.rsi < 40.0);
        }
    }

    #[test]
    fn test_mean_reversion_overbought() {
        // Create a series that rises sharply (overbought condition)
        let prices: Vec<f64> = (0..30).map(|i| 50.0 + i as f64 * 2.0).collect();
        let bars = make_bars(&prices);
        let result = mean_reversion_strategy(&bars, *prices.last().unwrap());

        if let Some(ref r) = result {
            assert_eq!(r.direction, TradeDirection::Short);
            assert!(r.rsi > 60.0);
        }
    }
}
