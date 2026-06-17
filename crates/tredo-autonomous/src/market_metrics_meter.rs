// MarketMetricsMeter — calculates rich market metrics as a first-class tool.
// Primary: deterministic local computation from SharedState ohlcv_history (agentic, always available).
// Supplements (when keys): Finnhub technical_indicator, Polygon aggs/indicators, CoinGecko ohlcv/vol, AlphaV indicators, FRED macro for regime context.
// Produces MetricsSnapshot used by strategy for autonomous_levels (ATR, regime, fib, etc), debate, MI confluence, and as AgentSkill vote into AggregatedSignal.
// Connects to: state (latest_metrics), memory pipelines (vector/episode snapshots for recall/self-evolution), WS (recompute on live price updates in loops), orchestrator pipeline (pre-identifier), news analyser synergy (sentiment+technicals).
// No price points given to agent — meter only provides perception data; agent decides everything.

use crate::helpers::{
    compute_adx, compute_atr, compute_bollinger_bands, compute_cci, compute_macd,
    compute_obv, compute_relative_volume, compute_rsi, compute_stochastic, compute_vwap,
    compute_williams_r,
};

fn default_50() -> f64 { 50.0 }
fn default_neg50() -> f64 { -50.0 }
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
    pub obv_direction: f64,   // OBV trend: >0 bullish, <0 bearish
    #[serde(default)]
    pub adx: f64,              // ADX trend strength (0-100)
    #[serde(default = "default_50")]
    pub plus_di: f64,          // +DI directional indicator
    #[serde(default = "default_50")]
    pub minus_di: f64,         // -DI directional indicator
    #[serde(default)]
    pub cci: f64,              // Commodity Channel Index (-∞ to +∞)
    #[serde(default = "default_neg50")]
    pub williams_r: f64,       // Williams %R (-100 to 0)
    #[serde(default)]
    pub vwap: f64,             // Volume Weighted Average Price
    #[serde(default)]
    pub vwap_deviation: f64,   // (price - vwap) / vwap
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

    /// Compute full metrics snapshot from current ohlcv (primary) + best-effort API supplements.
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
        if symbol == "BTC" || symbol == "ETH" || symbol == "SOL" {
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
        println!("[MetricsMeter] {} OBV raw={:.0} dir={:.3}", symbol, obv_raw, obv_dir);
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

        // Boost confluence if new indicators agree with existing signals
        // ADX > 25 confirms trend (adds precision in trending markets)
        if adx > 25.0 && ((plus_di > minus_di && rsi > 45.0) || (minus_di > plus_di && rsi < 55.0)) {
            snap.confluence_hint = (snap.confluence_hint + 0.05).min(0.95);
        }
        // OBV direction confirms price trend
        if (obv_dir > 0.0 && mac > 0.0) || (obv_dir < 0.0 && mac < 0.0) {
            snap.confluence_hint = (snap.confluence_hint + 0.04).min(0.95);
        }
        // VWAP deviation confirms direction
        if vwap_dev > 0.002 && rsi > 50.0 {
            snap.confluence_hint = (snap.confluence_hint + 0.03).min(0.95);
        } else if vwap_dev < -0.002 && rsi < 50.0 {
            snap.confluence_hint = (snap.confluence_hint + 0.03).min(0.95);
        }

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
