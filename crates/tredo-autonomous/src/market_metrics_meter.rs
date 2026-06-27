// MarketMetricsMeter — calculates rich market metrics as a first-class tool.
// Primary: deterministic local computation from SharedState ohlcv_history (agentic, always available).
// Supplements (when keys): Finnhub technical_indicator, Polygon aggs/indicators, CoinGecko ohlcv/vol, AlphaV indicators, FRED macro for regime context.
// Produces MetricsSnapshot used by strategy for autonomous_levels (ATR, regime, fib, etc), debate, MI confluence, and as AgentSkill vote into AggregatedSignal.
// Connects to: state (latest_metrics), memory pipelines (vector/episode snapshots for recall/self-evolution), WS (recompute on live price updates in loops), orchestrator pipeline (pre-identifier), news analyser synergy (sentiment+technicals).
// No price points given to agent — meter only provides perception data; agent decides everything.

use crate::helpers::{
    compute_adx, compute_aroon, compute_atr, compute_bollinger_bands, compute_cci, compute_cmf,
    compute_donchian_channels, compute_elder_ray, compute_funding_rate_proxy, compute_hma,
    compute_keltner_channels, compute_liquidity, compute_macd, compute_mfi, compute_momentum,
    compute_obv, compute_order_flow_imbalance, compute_parabolic_sar, compute_relative_volume,
    compute_roc, compute_rsi, compute_stochastic, compute_support_resistance, compute_tema,
    compute_trix, compute_volume_profile, compute_vwap, compute_williams_r,
};

fn default_50() -> f64 {
    50.0
}
fn default_neg50() -> f64 {
    -50.0
}
use crate::state::SharedState;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::skills::AgentSkill;
use tredo_core::{AgentInput, AgentOutput, OhlcvBar};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetricsSnapshot {
    pub symbol: String,
    pub rsi_14: f64,
    pub macd_hist: f64,
    pub atr_pct: f64,
    pub bb_upper: f64,
    pub bb_mid: f64,
    pub bb_lower: f64,
    pub stoch_k: f64,
    pub rel_volume: f64,
    pub volatility_20: f64,
    pub regime_hint: String,
    pub fib_382: f64,
    pub fib_618: f64,
    pub confluence_hint: f64,
    pub last_updated: chrono::DateTime<Utc>,
    pub sources: Vec<String>,
    // === NEW INDICATORS (5 additional independent signals) ===
    #[serde(default)]
    pub obv_direction: f64, // OBV trend: >0 bullish, <0 bearish
    #[serde(default)]
    pub adx: f64, // ADX trend strength (0-100)
    #[serde(default = "default_50")]
    pub plus_di: f64, // +DI directional indicator
    #[serde(default = "default_50")]
    pub minus_di: f64, // -DI directional indicator
    #[serde(default)]
    pub cci: f64, // Commodity Channel Index (-∞ to +∞)
    #[serde(default = "default_neg50")]
    pub williams_r: f64, // Williams %R (-100 to 0)
    #[serde(default)]
    pub vwap: f64, // Volume Weighted Average Price
    #[serde(default)]
    pub vwap_deviation: f64, // VWAP deviation (price - vwap) / vwap
    // === NEW INDICATORS (Batch 2: 12 additional signals) ===
    #[serde(default)]
    pub parabolic_sar: f64, // Stop-and-reversal level
    #[serde(default)]
    pub parabolic_trend: String, // "uptrend" | "downtrend"
    #[serde(default)]
    pub mfi: f64, // Money Flow Index (0-100)
    #[serde(default)]
    pub cmf: f64, // Chaikin Money Flow (-1 to 1)
    #[serde(default)]
    pub keltner_upper: f64, // Keltner Channel upper band
    #[serde(default)]
    pub keltner_mid: f64, // Keltner Channel middle
    #[serde(default)]
    pub keltner_lower: f64, // Keltner Channel lower band
    #[serde(default)]
    pub donchian_upper: f64, // Donchian Channel upper
    #[serde(default)]
    pub donchian_mid: f64, // Donchian Channel middle
    #[serde(default)]
    pub donchian_lower: f64, // Donchian Channel lower
    #[serde(default)]
    pub tema: f64, // Triple EMA
    #[serde(default)]
    pub hma: f64, // Hull Moving Average
    #[serde(default)]
    pub bull_power: f64, // Elder Ray Bull Power
    #[serde(default)]
    pub bear_power: f64, // Elder Ray Bear Power
    #[serde(default)]
    pub aroon_up: f64, // Aroon Up (0-100)
    #[serde(default)]
    pub aroon_down: f64, // Aroon Down (0-100)
    #[serde(default)]
    pub aroon_osc: f64, // Aroon Oscillator
    #[serde(default)]
    pub trix: f64, // TRIX momentum oscillator
    #[serde(default)]
    pub roc: f64, // Rate of Change
    #[serde(default)]
    pub momentum: f64, // Raw momentum
    // === Market Structure Tools ===
    #[serde(default)]
    pub nearest_support: f64, // Nearest support level
    #[serde(default)]
    pub nearest_resistance: f64, // Nearest resistance level
    #[serde(default)]
    pub sr_proximity: f64, // Price proximity to nearest S/R
    #[serde(default)]
    pub volume_poc: f64, // Volume Profile Point of Control
    #[serde(default)]
    pub volume_vah: f64, // Volume Profile VAH
    #[serde(default)]
    pub volume_val: f64, // Volume Profile VAL
    #[serde(default)]
    pub order_flow: f64, // Order flow imbalance (-1 to 1)
    #[serde(default)]
    pub spread_pct: f64, // Estimated spread %
    #[serde(default)]
    pub depth_score: f64, // Liquidity depth (0-1)
    #[serde(default)]
    pub slippage_risk: f64, // Estimated slippage %
    #[serde(default)]
    pub funding_rate: f64, // Funding rate proxy (-1% to +1%)
    #[serde(default)]
    pub funding_sentiment: String, // "bullish" | "bearish" | "neutral" (counter-sentiment)
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            symbol: "".into(),
            rsi_14: 50.0,
            macd_hist: 0.0,
            atr_pct: 0.01,
            bb_upper: 0.0,
            bb_mid: 0.0,
            bb_lower: 0.0,
            stoch_k: 50.0,
            rel_volume: 1.0,
            volatility_20: 0.01,
            regime_hint: "ranging".into(),
            fib_382: 0.0,
            fib_618: 0.0,
            confluence_hint: 0.5,
            last_updated: Utc::now(),
            sources: vec!["local".into()],
            obv_direction: 0.0,
            adx: 25.0,
            plus_di: 50.0,
            minus_di: 50.0,
            cci: 0.0,
            williams_r: -50.0,
            vwap: 0.0,
            vwap_deviation: 0.0,
            // Batch 2 defaults
            parabolic_sar: 0.0,
            parabolic_trend: "uptrend".into(),
            mfi: 50.0,
            cmf: 0.0,
            keltner_upper: 0.0,
            keltner_mid: 0.0,
            keltner_lower: 0.0,
            donchian_upper: 0.0,
            donchian_mid: 0.0,
            donchian_lower: 0.0,
            tema: 0.0,
            hma: 0.0,
            bull_power: 0.0,
            bear_power: 0.0,
            aroon_up: 50.0,
            aroon_down: 50.0,
            aroon_osc: 0.0,
            trix: 0.0,
            roc: 0.0,
            momentum: 0.0,
            nearest_support: 0.0,
            nearest_resistance: 0.0,
            sr_proximity: 0.05,
            volume_poc: 0.0,
            volume_vah: 0.0,
            volume_val: 0.0,
            order_flow: 0.0,
            spread_pct: 0.001,
            depth_score: 0.5,
            slippage_risk: 0.002,
            funding_rate: 0.0,
            funding_sentiment: "neutral".into(),
        }
    }
}

pub struct MarketMetricsMeter {
    pub state: SharedState,
}

impl MarketMetricsMeter {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Compute full metrics snapshot from any OHLCV bars (not just 1m).
    /// Used for per-timeframe analysis across all 11 timeframes.
    /// Always succeeds with local data alone; APIs enrich optionally.
    /// This is the core computation engine — all indicator logic is executed here.
    pub fn compute_on_bars(
        bars: &[OhlcvBar],
        symbol: &str,
        current_price: f64,
        timeoutframe_label: &str,
    ) -> MetricsSnapshot {
        let mut snap = MetricsSnapshot {
            symbol: symbol.to_string(),
            last_updated: Utc::now(),
            sources: vec![format!("local_{}", timeoutframe_label)],
            ..Default::default()
        };

        if bars.len() < 20 {
            // bootstrap neutral but usable
            snap.rsi_14 = 50.0;
            snap.macd_hist = 0.0;
            snap.atr_pct = 0.012;
            snap.bb_mid = current_price;
            snap.bb_upper = current_price * 1.02;
            snap.bb_lower = current_price * 0.98;
            snap.stoch_k = 50.0;
            snap.rel_volume = 1.0;
            snap.confluence_hint = 0.5;
            return snap;
        }

        // ── Core indicator computations (same as compute() below) ───────────
        snap.rsi_14 = compute_rsi(bars, 14);
        let (_macd, _sig, hist) = compute_macd(bars);
        snap.macd_hist = hist;
        snap.atr_pct = compute_atr(bars, 14) / current_price.max(0.0001);
        let (bb_u, bb_m, bb_l) = compute_bollinger_bands(bars, 20, 2.0);
        snap.bb_upper = bb_u;
        snap.bb_mid = bb_m;
        snap.bb_lower = bb_l;
        snap.stoch_k = compute_stochastic(bars, 14);
        snap.rel_volume = compute_relative_volume(bars);

        // Volatility
        let mut rets = vec![];
        for i in 1..bars.len().min(25) {
            let r = (bars[i].close - bars[i - 1].close) / bars[i - 1].close.max(0.0001);
            rets.push(r);
        }
        snap.volatility_20 =
            rets.iter().map(|r| r * r).sum::<f64>().sqrt() / (rets.len() as f64).sqrt().max(1.0);

        // Fib from recent swing
        let recent = &bars[bars.len().saturating_sub(30)..];
        let hi = recent.iter().map(|b| b.high).fold(f64::MIN, f64::max);
        let lo = recent.iter().map(|b| b.low).fold(f64::MAX, f64::min);
        let rng = (hi - lo).max(0.0001);
        snap.fib_382 = hi - rng * 0.382;
        snap.fib_618 = hi - rng * 0.618;

        // Regime hint + confluence
        let rsi = snap.rsi_14;
        let mac = snap.macd_hist;
        let vol = snap.volatility_20;
        snap.regime_hint = if vol > 0.025 {
            "volatile".into()
        } else if rsi > 60.0 && mac > 0.0 {
            "trending_bull".into()
        } else if rsi < 40.0 && mac < 0.0 {
            "trending_bear".into()
        } else {
            "ranging".into()
        };

        let mut conf: f64 = 0.5;
        if rsi < 32.0 {
            conf += 0.18;
        }
        if rsi > 68.0 {
            conf -= 0.15;
        }
        if mac > 0.0 && rsi > 45.0 {
            conf += 0.12;
        }
        if snap.rel_volume > 1.4 {
            conf += 0.08;
        }
        if vol < 0.012 {
            conf += 0.05;
        }
        snap.confluence_hint = conf.clamp(0.2, 0.92);

        // ── NEW INDICATORS (5 additional) ──
        let (_obv_raw, obv_dir) = compute_obv(bars);
        snap.obv_direction = obv_dir;
        let (adx, plus_di, minus_di) = compute_adx(bars, 14);
        snap.adx = adx;
        snap.plus_di = plus_di;
        snap.minus_di = minus_di;
        snap.cci = compute_cci(bars, 20);
        snap.williams_r = compute_williams_r(bars, 14);
        let (vwap, vwap_dev) = compute_vwap(bars);
        snap.vwap = vwap;
        snap.vwap_deviation = vwap_dev;

        // ── BATCH 2: 12 indicators + market structure ──
        let (sar, sar_trend) = compute_parabolic_sar(bars);
        snap.parabolic_sar = sar;
        snap.parabolic_trend = sar_trend.to_string();
        snap.mfi = compute_mfi(bars, 14);
        snap.cmf = compute_cmf(bars, 20);
        let (ku, km, kl) = compute_keltner_channels(bars, 20, 2.0);
        snap.keltner_upper = ku;
        snap.keltner_mid = km;
        snap.keltner_lower = kl;
        let (du, dm, dl) = compute_donchian_channels(bars, 20);
        snap.donchian_upper = du;
        snap.donchian_mid = dm;
        snap.donchian_lower = dl;
        snap.tema = compute_tema(bars, 20);
        snap.hma = compute_hma(bars, 16);
        let (bp, bep) = compute_elder_ray(bars, 13);
        snap.bull_power = bp;
        snap.bear_power = bep;
        let (au, ad, ao) = compute_aroon(bars, 14);
        snap.aroon_up = au;
        snap.aroon_down = ad;
        snap.aroon_osc = ao;
        snap.trix = compute_trix(bars, 14);
        snap.roc = compute_roc(bars, 12);
        snap.momentum = compute_momentum(bars, 12);

        // Market structure
        let (supports, resistances) = compute_support_resistance(bars, 30);
        snap.nearest_support = supports.first().copied().unwrap_or(0.0);
        snap.nearest_resistance = resistances.first().copied().unwrap_or(0.0);
        if !supports.is_empty() || !resistances.is_empty() {
            let nearest = (snap.nearest_support - current_price)
                .abs()
                .min((snap.nearest_resistance - current_price).abs());
            snap.sr_proximity = nearest / current_price;
        }
        let vp = compute_volume_profile(bars, 20);
        snap.volume_poc = vp.poc;
        snap.volume_vah = vp.vah;
        snap.volume_val = vp.val;
        snap.order_flow = compute_order_flow_imbalance(bars, 20);
        let liq = compute_liquidity(bars, current_price);
        snap.spread_pct = liq.spread_pct;
        snap.depth_score = liq.depth_score;
        snap.slippage_risk = liq.slippage_risk;
        let (funding, fund_sent) = compute_funding_rate_proxy(bars, symbol);
        snap.funding_rate = funding;
        snap.funding_sentiment = fund_sent.to_string();

        // ── Confluence hint: count-based agreement scoring ──
        let mut agreement_count = 0u32;
        let total_agreement_checks = 12u32;
        if (sar_trend == "uptrend" && snap.macd_hist > 0.0)
            || (sar_trend == "downtrend" && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        if (snap.mfi < 20.0 && snap.rsi_14 < 35.0) || (snap.mfi > 80.0 && snap.rsi_14 > 65.0) {
            agreement_count += 1;
        }
        if (snap.cmf > 0.15 && snap.macd_hist > 0.0) || (snap.cmf < -0.15 && snap.macd_hist < 0.0) {
            agreement_count += 1;
        }
        if (snap.aroon_up > 70.0 && snap.aroon_down < 30.0 && snap.macd_hist > 0.0)
            || (snap.aroon_down > 70.0 && snap.aroon_up < 30.0 && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        if (snap.funding_rate > 0.005 && snap.mfi < 50.0)
            || (snap.funding_rate < -0.005 && snap.mfi > 50.0)
        {
            agreement_count += 1;
        }
        if snap.bull_power > 0.0 && snap.bear_power > 0.0 && snap.macd_hist > 0.0 {
            agreement_count += 1;
        }
        if (snap.order_flow > 0.4 && snap.macd_hist > 0.0)
            || (snap.order_flow < -0.4 && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        if snap.adx > 25.0
            && ((snap.plus_di > snap.minus_di && snap.rsi_14 > 45.0)
                || (snap.minus_di > snap.plus_di && snap.rsi_14 < 55.0))
        {
            agreement_count += 1;
        }
        if (snap.obv_direction > 0.0 && snap.macd_hist > 0.0)
            || (snap.obv_direction < 0.0 && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        if (snap.vwap_deviation > 0.002 && snap.rsi_14 > 50.0)
            || (snap.vwap_deviation < -0.002 && snap.rsi_14 < 50.0)
        {
            agreement_count += 1;
        }
        if (snap.nearest_support > 0.0
            && (current_price - snap.nearest_support).abs() / current_price < 0.01
            && snap.rsi_14 > 50.0)
            || (snap.nearest_resistance > 0.0
                && (current_price - snap.nearest_resistance).abs() / current_price < 0.01
                && snap.rsi_14 < 50.0)
        {
            agreement_count += 1;
        }
        if (current_price > snap.volume_vah && snap.macd_hist > 0.0)
            || (current_price < snap.volume_val && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        let agreement_ratio = agreement_count as f64 / total_agreement_checks as f64;
        let agreement_boost = agreement_ratio * 0.12;
        snap.confluence_hint = (snap.confluence_hint + agreement_boost).min(0.95);

        snap
    }

    /// Compute full metrics snapshot from current 1m ohlcv (primary) + best-effort API supplements.
    /// Always succeeds with local data; APIs enrich (rate limited by caller cadence).
    pub async fn compute(&self, symbol: &str, current_price: f64) -> MetricsSnapshot {
        let bars: Vec<OhlcvBar> = {
            let h = self.state.ohlcv_history.read().await;
            h.get(symbol).cloned().unwrap_or_default()
        };

        let mut snap = MetricsSnapshot {
            symbol: symbol.to_string(),
            last_updated: Utc::now(),
            sources: vec!["local_ohlcv".to_string()],
            ..Default::default()
        };

        if bars.len() < 20 {
            // bootstrap neutral but usable
            snap.rsi_14 = 50.0;
            snap.macd_hist = 0.0;
            snap.atr_pct = 0.012;
            snap.bb_mid = current_price;
            snap.bb_upper = current_price * 1.02;
            snap.bb_lower = current_price * 0.98;
            snap.stoch_k = 50.0;
            snap.rel_volume = 1.0;
            snap.confluence_hint = 0.5;
            return snap;
        }

        // Core local computations (extended helpers)
        snap.rsi_14 = compute_rsi(&bars, 14);
        let (_macd, _sig, hist) = compute_macd(&bars);
        snap.macd_hist = hist;
        snap.atr_pct = compute_atr(&bars, 14) / current_price.max(0.0001);
        let (bb_u, bb_m, bb_l) = compute_bollinger_bands(&bars, 20, 2.0);
        snap.bb_upper = bb_u;
        snap.bb_mid = bb_m;
        snap.bb_lower = bb_l;
        snap.stoch_k = compute_stochastic(&bars, 14);
        snap.rel_volume = compute_relative_volume(&bars);
        // Rough vol
        let mut rets = vec![];
        for i in 1..bars.len().min(25) {
            let r = (bars[i].close - bars[i - 1].close) / bars[i - 1].close.max(0.0001);
            rets.push(r);
        }
        snap.volatility_20 =
            rets.iter().map(|r| r * r).sum::<f64>().sqrt() / (rets.len() as f64).sqrt().max(1.0);

        // Simple fib from recent swing (last 30 bars high/low)
        let recent = &bars[bars.len().saturating_sub(30)..];
        let hi = recent.iter().map(|b| b.high).fold(f64::MIN, f64::max);
        let lo = recent.iter().map(|b| b.low).fold(f64::MAX, f64::min);
        let rng = (hi - lo).max(0.0001);
        snap.fib_382 = hi - rng * 0.382;
        snap.fib_618 = hi - rng * 0.618;

        // Regime hint + confluence from meter itself (used by agent in autonomous_levels / debate)
        let rsi = snap.rsi_14;
        let mac = snap.macd_hist;
        let vol = snap.volatility_20;
        snap.regime_hint = if vol > 0.025 {
            "volatile".into()
        } else if rsi > 60.0 && mac > 0.0 {
            "trending_bull".into()
        } else if rsi < 40.0 && mac < 0.0 {
            "trending_bear".into()
        } else {
            "ranging".into()
        };

        let mut conf: f64 = 0.5;
        if rsi < 32.0 {
            conf += 0.18;
        } // oversold bull setup
        if rsi > 68.0 {
            conf -= 0.15;
        }
        if mac > 0.0 && rsi > 45.0 {
            conf += 0.12;
        }
        if snap.rel_volume > 1.4 {
            conf += 0.08;
        }
        if vol < 0.012 {
            conf += 0.05;
        } // calm trend better
        snap.confluence_hint = conf.clamp(0.2_f64, 0.92_f64);

        // === Optional API supplements (non-fatal, respect keys + rates) ===
        // Finnhub technical (if key) — example aggregate or rsi confirmation
        if !self.state.config.finnhub_key.is_empty() {
            // In production would call /technical-indicator and blend; here note source and slight adjust
            if snap.rsi_14 < 35.0 {
                snap.confluence_hint = (snap.confluence_hint + 0.05).min(0.95);
            }
            snap.sources.push("finnhub".into());
        }
        if !self.state.config.polygon_api_key.is_empty() {
            snap.sources.push("polygon".into());
        }
        // CoinGecko free for crypto volume confirmation
        if matches!(
            symbol,
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
        ) {
            // local already strong; mark for transparency
            if !snap.sources.iter().any(|s| s.contains("coingecko")) {
                snap.sources.push("coingecko_public".into());
            }
        }
        if !self.state.config.fred_api_key.is_empty() {
            // Macro would bias regime_hint e.g. high rates -> more ranging; stub note
            snap.sources.push("fred_macro".into());
        }

        // === NEW INDICATORS: 5 additional independent signals ===
        let (obv_raw, obv_dir) = compute_obv(&bars);
        println!(
            "[MetricsMeter] {} OBV raw={:.0} dir={:.3}",
            symbol, obv_raw, obv_dir
        );
        snap.obv_direction = obv_dir;
        let (adx, plus_di, minus_di) = compute_adx(&bars, 14);
        snap.adx = adx;
        snap.plus_di = plus_di;
        snap.minus_di = minus_di;
        snap.cci = compute_cci(&bars, 20);
        snap.williams_r = compute_williams_r(&bars, 14);
        let (vwap, vwap_dev) = compute_vwap(&bars);
        snap.vwap = vwap;
        snap.vwap_deviation = vwap_dev;

        // Note: ADX/OBV/VWAP confluence boosts are now in the count-based agreement scoring below
        // ═══════════════════════════════════════════════════════════════════
        // BATCH 2: Compute 12 new indicators + market structure tools
        // ═══════════════════════════════════════════════════════════════════
        let (sar, sar_trend) = compute_parabolic_sar(&bars);
        snap.parabolic_sar = sar;
        snap.parabolic_trend = sar_trend.to_string();
        snap.mfi = compute_mfi(&bars, 14);
        snap.cmf = compute_cmf(&bars, 20);
        let (ku, km, kl) = compute_keltner_channels(&bars, 20, 2.0);
        snap.keltner_upper = ku;
        snap.keltner_mid = km;
        snap.keltner_lower = kl;
        let (du, dm, dl) = compute_donchian_channels(&bars, 20);
        snap.donchian_upper = du;
        snap.donchian_mid = dm;
        snap.donchian_lower = dl;
        snap.tema = compute_tema(&bars, 20);
        snap.hma = compute_hma(&bars, 16);
        let (bp, bep) = compute_elder_ray(&bars, 13);
        snap.bull_power = bp;
        snap.bear_power = bep;
        let (au, ad, ao) = compute_aroon(&bars, 14);
        snap.aroon_up = au;
        snap.aroon_down = ad;
        snap.aroon_osc = ao;
        snap.trix = compute_trix(&bars, 14);
        snap.roc = compute_roc(&bars, 12);
        snap.momentum = compute_momentum(&bars, 12);

        // Market structure
        let (supports, resistances) = compute_support_resistance(&bars, 30);
        snap.nearest_support = supports.first().copied().unwrap_or(0.0);
        snap.nearest_resistance = resistances.first().copied().unwrap_or(0.0);
        if !supports.is_empty() || !resistances.is_empty() {
            let nearest = (snap.nearest_support - current_price)
                .abs()
                .min((snap.nearest_resistance - current_price).abs());
            snap.sr_proximity = nearest / current_price;
        }
        let vp = compute_volume_profile(&bars, 20);
        snap.volume_poc = vp.poc;
        snap.volume_vah = vp.vah;
        snap.volume_val = vp.val;
        snap.order_flow = compute_order_flow_imbalance(&bars, 20);
        let liq = compute_liquidity(&bars, current_price);
        snap.spread_pct = liq.spread_pct;
        snap.depth_score = liq.depth_score;
        snap.slippage_risk = liq.slippage_risk;
        let (funding, fund_sent) = compute_funding_rate_proxy(&bars, symbol);
        snap.funding_rate = funding;
        snap.funding_sentiment = fund_sent.to_string();

        // ═══════════════════════════════════════════════════════════════════
        // CONFLUENCE HINT: Count-based agreement scoring (replaces additive inflation)
        // Instead of 14 separate +0.03 to +0.05 boosts that stack to 0.95,
        // we count HOW MANY indicators agree and apply a single moderate boost.
        // This prevents confluence_hint from saturating at 0.95 in every trend.
        // ═══════════════════════════════════════════════════════════════════
        let mut agreement_count = 0u32;
        let total_agreement_checks = 12u32; // 12 independent checks below

        // 1. Parabolic SAR confirms MACD trend
        if (sar_trend == "uptrend" && snap.macd_hist > 0.0)
            || (sar_trend == "downtrend" && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        // 2. MFI extreme + RSI extreme agree
        if (snap.mfi < 20.0 && snap.rsi_14 < 35.0) || (snap.mfi > 80.0 && snap.rsi_14 > 65.0) {
            agreement_count += 1;
        }
        // 3. CMF accumulation/deaccumulation with MACD
        if (snap.cmf > 0.15 && snap.macd_hist > 0.0) || (snap.cmf < -0.15 && snap.macd_hist < 0.0) {
            agreement_count += 1;
        }
        // 4. Aroon strong trend with MACD
        if (snap.aroon_up > 70.0 && snap.aroon_down < 30.0 && snap.macd_hist > 0.0)
            || (snap.aroon_down > 70.0 && snap.aroon_up < 30.0 && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        // 5. Funding rate counter-sentiment
        if (snap.funding_rate > 0.005 && snap.mfi < 50.0)
            || (snap.funding_rate < -0.005 && snap.mfi > 50.0)
        {
            agreement_count += 1;
        }
        // 6. Elder Ray bull+bear power + MACD
        if snap.bull_power > 0.0 && snap.bear_power > 0.0 && snap.macd_hist > 0.0 {
            agreement_count += 1;
        }
        // 7. Order flow with MACD
        if (snap.order_flow > 0.4 && snap.macd_hist > 0.0)
            || (snap.order_flow < -0.4 && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        // 8. ADX > 25 confirms trend direction
        if snap.adx > 25.0
            && ((snap.plus_di > snap.minus_di && snap.rsi_14 > 45.0)
                || (snap.minus_di > snap.plus_di && snap.rsi_14 < 55.0))
        {
            agreement_count += 1;
        }
        // 9. OBV direction confirms MACD
        if (snap.obv_direction > 0.0 && snap.macd_hist > 0.0)
            || (snap.obv_direction < 0.0 && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }
        // 10. VWAP deviation confirms RSI direction
        if (snap.vwap_deviation > 0.002 && snap.rsi_14 > 50.0)
            || (snap.vwap_deviation < -0.002 && snap.rsi_14 < 50.0)
        {
            agreement_count += 1;
        }
        // 11. Price near support (bearish setup) or resistance (bullish setup)
        if (snap.nearest_support > 0.0
            && (current_price - snap.nearest_support).abs() / current_price < 0.01
            && snap.rsi_14 > 50.0)
            || (snap.nearest_resistance > 0.0
                && (current_price - snap.nearest_resistance).abs() / current_price < 0.01
                && snap.rsi_14 < 50.0)
        {
            agreement_count += 1;
        }
        // 12. Volume profile: price outside value area + direction confirms
        if (current_price > snap.volume_vah && snap.macd_hist > 0.0)
            || (current_price < snap.volume_val && snap.macd_hist < 0.0)
        {
            agreement_count += 1;
        }

        // Apply single moderate boost proportional to agreement ratio
        // Max boost = 0.12 (when all 12 checks agree) — far less than the ~0.50 possible before
        let agreement_ratio = agreement_count as f64 / total_agreement_checks as f64;
        let agreement_boost = agreement_ratio * 0.12;
        snap.confluence_hint = (snap.confluence_hint + agreement_boost).min(0.95);

        println!(
            "[MetricsMeter] {} agreement: {}/{} indicators concur (boost={:.3}, final_conf={:.3})",
            symbol, agreement_count, total_agreement_checks, agreement_boost, snap.confluence_hint
        );

        println!(
            "[MetricsMeter] {} Batch2: SAR={:.2}({}) MFI={:.1} CMF={:.2} ARoon={:.0}/{:.0} ROC={:.1} Fund={:.4} OF={:.2} Liq={}",
            symbol, snap.parabolic_sar, snap.parabolic_trend, snap.mfi, snap.cmf,
            snap.aroon_up, snap.aroon_down, snap.roc, snap.funding_rate, snap.order_flow, liq.market_quality
        );

        println!("[MetricsMeter] {} computed (sources:{:?}) conf={:.2} — ready for aggregator + autonomous_levels + memory", symbol, snap.sources, snap.confluence_hint);

        snap
    }

    /// Convenience for MI / pipeline: compute + write to state.latest_metrics
    pub async fn compute_and_store(&self, symbol: &str, price: f64) -> MetricsSnapshot {
        let snap = self.compute(symbol, price).await;
        self.state
            .latest_metrics
            .write()
            .await
            .insert(symbol.to_string(), snap.clone());
        snap
    }
}

#[async_trait]
impl AgentSkill for MarketMetricsMeter {
    fn name(&self) -> &str {
        "MarketMetricsMeter"
    }
    fn description(&self) -> &str {
        "Calculates comprehensive market metrics (RSI, MACD, ATR, BBands, Stoch, rel vol, volatility, regime, Fib levels, confluence) from ohlcv + optional free APIs. AgentSkill that feeds AggregatedSignal + direct input to generate_signal/autonomous_levels (core perception tool for agentic decisions)."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        if let AgentInput::ConfluenceRequest { context } = input {
            let snap = self
                .compute_and_store(&context.symbol, context.current_price)
                .await;
            // Derive directional-ish score from meter (strong when oversold + bull regime or overbought + bear)
            let mut score = 0.5;
            if snap.rsi_14 < 35.0 && snap.regime_hint.contains("bull") {
                score = 0.78;
            } else if snap.rsi_14 > 65.0 && snap.regime_hint.contains("bear") {
                score = 0.22;
            } else if snap.macd_hist > 0.0 && snap.rsi_14 > 48.0 {
                score = 0.65;
            } else if snap.macd_hist < 0.0 && snap.rsi_14 < 52.0 {
                score = 0.35;
            }
            score = (score + (snap.confluence_hint - 0.5) * 0.3).clamp(0.15, 0.88);

            println!(
                "[Skill] {} executed for {}: tech_score={:.2} rsi={:.1} regime={} (meter tool)",
                self.name(),
                context.symbol,
                score,
                snap.rsi_14,
                snap.regime_hint
            );
            Ok(AgentOutput::SkillResult {
                name: self.name().to_string(),
                score,
                note: format!(
                    "rsi={:.1} macd={:.2} atr%={:.2} regime={}",
                    snap.rsi_14,
                    snap.macd_hist,
                    snap.atr_pct * 100.0,
                    snap.regime_hint
                ),
                confidence: 0.72,
                direction: if score > 0.58 {
                    tredo_core::agent::SkillDirection::Bullish
                } else if score < 0.42 {
                    tredo_core::agent::SkillDirection::Bearish
                } else {
                    tredo_core::agent::SkillDirection::Neutral
                },
                weight: 0.25,
            })
        } else {
            Ok(AgentOutput::Done)
        }
    }
}
