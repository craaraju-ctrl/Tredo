# TRED Indicator & Rule Inventory

> Single source of truth for every indicator, rule, threshold, and signal weight in the TRED autonomous trading system.
> Last updated: June 2026

---

## Table of Contents

1. [Technical Indicators (26)](#1-technical-indicators-26)
2. [Candlestick Patterns (27)](#2-candlestick-patterns-27)
3. [Agent Skills (14)](#3-agent-skills-14)
4. [Debate Layer Agents (7)](#4-debate-layer-agents-7)
5. [Risk & Discipline Rules (14)](#5-risk--discipline-rules-14)
6. [Regime-Adaptive Thresholds](#6-regime-adaptive-thresholds)
7. [Debate Layer Signal Weights](#7-debate-layer-signal-weights)
8. [Position Sizing Multipliers](#8-position-sizing-multipliers)
9. [Learning & Evolution Systems (4)](#9-learning--evolution-systems-4)
10. [5-Layer Pipeline Architecture](#10-5-layer-pipeline-architecture)
11. [Priority-Based Conflict Resolution](#11-priority-based-conflict-resolution)
12. [SuperIntelligence Decision Layer](#12-superintelligence-decision-layer-new)

---

## 1. Technical Indicators (26)

| # | Indicator | Function | Location | Returns | Notes |
|---|-----------|----------|----------|---------|-------|
| 1 | **RSI** (14) | `compute_rsi()` | `helpers.rs` | 0–100 | Wilder's method, neutral=50 |
| 2 | **MACD** (12,26,9) | `compute_macd()` | `helpers.rs` | (macd, signal, hist) | Simplified signal approximation |
| 3 | **ATR** (14) | `compute_atr()` | `helpers.rs` | f64 | True Range with Wilder's smoothing |
| 4 | **Bollinger Bands** (20, 2σ) | `compute_bollinger_bands()` | `helpers.rs` | (upper, mid, lower) | Standard deviation bands |
| 5 | **Stochastic** (14) | `compute_stochastic()` | `helpers.rs` | 0–100 | %K oscillator |
| 6 | **Fibonacci Levels** | `compute_fib_levels()` | `helpers.rs` | (38.2%, 61.8%) | Retracement from swing |
| 7 | **Relative Volume** | `compute_relative_volume()` | `helpers.rs` | 0.3–3.0 | Current vs average volume |
| 8 | **OBV** | `compute_obv()` | `helpers.rs` | (value, direction) | Single-pass, normalized slope |
| 9 | **ADX** (14) | `compute_adx()` | `helpers.rs` | (adx, +DI, -DI) | Simplified Wilder's smoothing |
| 10 | **CCI** (20) | `compute_cci()` | `helpers.rs` | f64 (unbounded) | Typical price vs mean deviation |
| 11 | **Williams %R** (14) | `compute_williams_r()` | `helpers.rs` | -100 to 0 | Overbought >-20, oversold <-80 |
| 12 | **VWAP** | `compute_vwap()` | `helpers.rs` | (price, deviation%) | Rolling (not session-reset) |
| 13 | **Pivot Points** | `calculate_pivot_points()` | `disciplined_core.rs` | PivotLevels | Classic, Woodie, Fibonacci |
| 14 | **EMA** | `compute_ema()` | `helpers.rs` | f64 | Internal, used by MACD |
| **15** | **Parabolic SAR** | `compute_parabolic_sar()` | `helpers.rs` | (sar, trend) | Stop-and-reversal, trend-following |
| **16** | **MFI** (14) | `compute_mfi()` | `helpers.rs` | 0–100 | Volume-weighted RSI. >80 overbought, <20 oversold |
| **17** | **CMF** (20) | `compute_cmf()` | `helpers.rs` | -1 to 1 | Chaikin Money Flow. >0.1 accumulation, <-0.1 distribution |
| **18** | **Keltner Channels** | `compute_keltner_channels()` | `helpers.rs` | (upper, mid, lower) | ATR-based volatility envelopes |
| **19** | **Donchian Channels** | `compute_donchian_channels()` | `helpers.rs` | (upper, mid, lower) | Highest-high/lowest-low breakout bands |
| **20** | **TEMA** | `compute_tema()` | `helpers.rs` | f64 | Triple EMA — faster, less lag than EMA |
| **21** | **HMA** | `compute_hma()` | `helpers.rs` | f64 | Hull Moving Average — extremely responsive |
| **22** | **Elder Ray** | `compute_elder_ray()` | `helpers.rs` | (bull_power, bear_power) | Bull/Bear Power vs EMA(13) |
| **23** | **Aroon** (14) | `compute_aroon()` | `helpers.rs` | (up, down, osc) | Time since HH/LL. Up>70 = strong uptrend |
| **24** | **TRIX** | `compute_trix()` | `helpers.rs` | f64 | Triple-smoothed ROC momentum. Good for divergences |
| **25** | **ROC** (12) | `compute_roc()` | `helpers.rs` | % | Rate of Change — simple but effective momentum |
| **26** | **Momentum** | `compute_momentum()` | `helpers.rs` | f64 | Raw price difference over N periods |

> **Batch 2**: 12 new indicators added (Parabolic SAR through Momentum). Total expanded from 14 → 26.
> Information Ratio scaling: √26 ≈ 5.1x vs single-signal system.

### Market Structure Tools (No Indicator Number — Auxiliary)

| Tool | Function | Location | Purpose |
|------|----------|----------|---------|
| **Support/Resistance** | `compute_support_resistance()` | `helpers.rs` | Swing high/low clustering for S/R zones |
| **Volume Profile** | `compute_volume_profile()` | `helpers.rs` | POC, VAH, VAL from volume distribution |
| **Order Flow** | `compute_order_flow_imbalance()` | `helpers.rs` | Buy/sell pressure from bar close position + volume |
| **Liquidity** | `compute_liquidity()` | `helpers.rs` | Spread, depth, slippage risk estimation |
| **Funding Rate** | `compute_funding_rate_proxy()` | `helpers.rs` | Crypto perp funding proxy (counter-sentiment) |

---

## 2. Candlestick Patterns (27)

| # | Pattern | Function | Bars Required | Type |
|---|---------|----------|---------------|------|
| 1 | Doji | `detect_doji()` | 1 | Neutral |
| 2 | Hammer | `detect_hammer()` | 1 | Bullish reversal |
| 3 | Shooting Star | `detect_shooting_star()` | 1 | Bearish reversal |
| 4 | Marubozu | `detect_marubozu()` | 1 | Continuation |
| 5 | Spinning Top | `detect_spinning_top()` | 1 | Neutral |
| 6 | **Dragonfly Doji** | `detect_dragonfly_doji()` | 1 | **Bullish reversal** |
| 7 | **Gravestone Doji** | `detect_gravestone_doji()` | 1 | **Bearish reversal** |
| 8 | **Bullish Belt Hold** | `detect_belt_hold()` | 1 | **Bullish continuation** |
| 9 | **Bearish Belt Hold** | `detect_belt_hold()` | 1 | **Bearish continuation** |
| 10 | Bullish Engulfing | `detect_bullish_engulfing()` | 2 | Bullish reversal |
| 11 | Bearish Engulfing | `detect_bearish_engulfing()` | 2 | Bearish reversal |
| 12 | Bullish Harami | `detect_bullish_harami()` | 2 | Bullish reversal |
| 13 | Bearish Harami | `detect_bearish_harami()` | 2 | Bearish reversal |
| 14 | Piercing Line | `detect_piercing_line()` | 2 | Bullish reversal |
| 15 | Dark Cloud Cover | `detect_dark_cloud_cover()` | 2 | Bearish reversal |
| 16 | **Tweezer Top** | `detect_tweezer_top()` | 2 | **Bearish reversal** |
| 17 | **Tweezer Bottom** | `detect_tweezer_bottom()` | 2 | **Bullish reversal** |
| 18 | **Harami Cross** | `detect_harami_cross()` | 2 | **Reversal** |
| 19 | **Bullish Kicking** | `detect_kicking()` | 2 | **Bullish reversal** |
| 20 | **Bearish Kicking** | `detect_kicking()` | 2 | **Bearish reversal** |
| 21 | Morning Star | `detect_morning_star()` | 3 | Bullish reversal |
| 22 | Evening Star | `detect_evening_star()` | 3 | Bearish reversal |
| 23 | Three White Soldiers | `detect_three_white_soldiers()` | 3 | Bullish continuation |
| 24 | Three Black Crows | `detect_three_black_crows()` | 3 | Bearish continuation |
| 25 | **Morning Doji Star** | `detect_morning_doji_star()` | 3 | **Bullish reversal** |
| 26 | **Evening Doji Star** | `detect_evening_doji_star()` | 3 | **Bearish reversal** |
| 27 | **Abandoned Baby** | `detect_abandoned_baby()` | 3 | **Reversal** |
| 28 | **Rising Three Methods** | `detect_rising_three_methods()` | 5 | **Bullish continuation** |
| 29 | **Falling Three Methods** | `detect_falling_three_methods()` | 5 | **Bearish continuation** |
| 30 | Multi-TF Confirmation | `detect_patterns_multi_tf()` | Cross-timeframe | Combined |

> **New patterns**: Dragonfly/Gravestone Doji, Belt Hold, Tweezer Top/Bottom, Harami Cross, Kicking, Morning/Evening Doji Star, Abandoned Baby, Rising/Falling Three Methods. Total: 27 patterns (was 16).

---

## 3. Agent Skills (14)

| # | Skill | File | Default Weight | Domain |
|---|-------|------|----------------|--------|
| 1 | SentimentAnalyzer | `sentiment_analyzer.rs` | 0.12 | News/market sentiment scoring |
| 2 | VolatilityCalculator | `volatility_calculator.rs` | 0.08 | Vol measurement + regime detection |
| 3 | RegimeDetector | `regime_detector.rs` | 0.10 | Market regime classification |
| 4 | CorrelationChecker | `correlation_checker.rs` | 0.05 | Cross-symbol correlation risk |
| 5 | OnChainData | `on_chain_data.rs` | 0.06 | Blockchain mempool/hashrate/volume |
| 6 | TrainedMemorySkill | `skills.rs` | 0.10 | Vector memory recall |
| 7 | PatternRetriever | `pattern_retriever.rs` | 0.08 | Historical pattern matching |
| 8 | NewsAnalyser | `news_analyser.rs` | 0.12 | Multi-source news sentiment |
| 9 | MarketMetricsMeter | `market_metrics_meter.rs` | 0.12 | Rich indicator bundle (26 indicators) |
| **10** | **SupportResistance** | `support_resistance.rs` | **0.08** | **Swing high/low S/R level detection** |
| **11** | **VolumeProfile** | `volume_profile.rs` | **0.06** | **POC, VAH, VAL from volume distribution** |
| **12** | **OrderFlow** | `order_flow.rs` | **0.07** | **Buy/sell pressure imbalance** |
| **13** | **FundingRate** | `funding_rate.rs` | **0.05** | **Crypto perp funding counter-sentiment** |
| **14** | **Liquidity** | `liquidity.rs` | **0.05** | **Market quality, spread, slippage risk** |

> **Total weights: ~1.0** — Adjusted by MetaControl based on regime-specific accuracy.
> **New skills (5)**: SupportResistance, VolumeProfile, OrderFlow, FundingRate, Liquidity.
> **Rebalanced weights**: Reduced legacy skills slightly to accommodate new ones while maintaining ~1.0 total.

---

## 4. Debate Layer Agents (7)

| # | Agent | Role | Evidence Factors | Decision Power |
|---|-------|------|------------------|----------------|
| 1 | **ProposerAgent** | Entry case builder | 8 signals (skill, regime, RSI, patterns, news, memory, vol, MACD) | Advisory only |
| 2 | **CriticAgent** | Challenge generator | 5 factors (correlation, vol-price divergence, regime mismatch, RSI exhaustion, memory fakeouts) | Advisory only |
| 3 | **RiskAgent** | Risk assessment | 5 factors (vol vs regime, expansion, heat, losses, regime) | Advisory only |
| 4 | **HistorianAgent** | Episode interpretation | Counts profitable/losing episodes, avg_regret, patterns | Advisory only |
| 5 | **BullTeam** | Strongest bullish case | 12 factors (skill, regime, RSI, news, patterns, MACD, memory, OBV, ADX, CCI, Williams %R, VWAP) | Advisory only |
| 6 | **BearTeam** | Strongest bearish case + SELL | 11 factors (skill, regime, heat, losses, RSI, news, OBV, ADX, CCI, Williams %R, VWAP) | Advisory only |
| 7 | **Judge/Adjudicator** | Final authority | Debate quality evaluation only (see §10) | **Final decision** |

> **Key principle:** All 6 debate agents are ADVISORY ONLY. They provide evidence + confidence scores.
> Only the Judge has decision-making power. Debate agents can recommend BUY/HOLD/SELL but never BLOCK.

---

## 5. Risk & Discipline Rules (14)

| # | Rule | Location | Threshold | Priority | Enforcement |
|---|------|----------|-----------|----------|-------------|
| 1 | Daily Drawdown | `hard_rules_gate.rs` | **2% hard limit** | Critical | HardRulesGate (Layer 1) |
| 2 | Trading Enabled | `hard_rules_gate.rs` | Boolean flag | Critical | HardRulesGate (Layer 1) |
| 3 | Red Folder Discipline | `hard_rules_gate.rs` | No trading on high-impact events | Critical | HardRulesGate (Layer 1) |
| 4 | Session Timing | `hard_rules_gate.rs` | London/NY hours (crypto bypasses) | Critical | HardRulesGate (Layer 1) |
| 5 | Portfolio Heat Limit | `hard_rules_gate.rs` | **10%** | High | HardRulesGate (Layer 1) |
| 6 | Loss Circuit Breaker | `hard_rules_gate.rs` | 4+ consecutive losses | High | HardRulesGate (Layer 1) |
| 7 | Max Daily Trades | `hard_rules_gate.rs` | 8 total/day | High | HardRulesGate (Layer 1) |
| 8 | Cooldown | `hard_rules_gate.rs` | 30 minutes between same-symbol trades | High | HardRulesGate (Layer 1) |
| 9 | Regime Safety | `hard_rules_gate.rs` | No BUY in bear regime with <60% confluence | Medium | HardRulesGate (Layer 1) |
| 10 | Min Confluence | `hard_rules_gate.rs` | **Regime-adaptive** (0.50–0.85) | Medium | HardRulesGate (Layer 1) |
| 11 | Max Positions/Symbol | `hard_rules_gate.rs` | 3 per symbol | Low | HardRulesGate (warning only) |
| 12 | Max Total Positions | `hard_rules_gate.rs` | 10 total | Low | HardRulesGate (warning only) |
| 13 | Regime Consistency | `orchestrator_pipeline.rs` | Price trend vs declared regime | — | WFA Gate (Layer 2) |
| 14 | Confidence Minimum | `debate_layer.rs` | **Regime-adaptive** (0.40–0.75) | — | Judge (Layer 4, debate quality) |

---

## 6. Regime-Adaptive Thresholds

### 6.1 Market Regimes

| Regime | Description | Confidence Multiplier |
|--------|-------------|----------------------|
| TrendingBull | Strong upward trend | 1.0 |
| TrendingBear | Strong downward trend | 0.7 |
| Ranging | Sideways/choppy | 0.8 |
| Volatile | High volatility | 0.6 |
| LowLiquidity | Low volume/participation | 0.6 |

### 6.2 Threshold Tables

| Threshold | TrendingBull | TrendingBear | Ranging | Volatile | LowLiquidity | Enforced By |
|-----------|-------------|--------------|---------|----------|--------------|-------------|
| **Min Confluence** | 0.50 | 0.80 | 0.70 | 0.75 | 0.85 | HardRulesGate (Medium) |
| **Debate Buy Score** | 0.25 | 0.55 | 0.40 | 0.50 | 0.60 | DebateLayer Round 1 |
| **Vol Block** | 0.04 | 0.02 | 0.03 | 0.05 | 0.02 | DebateLayer |
| **Judge Min Confidence** | 0.40 | 0.60 | 0.50 | 0.65 | 0.75 | Judge (Layer 4) |
| **Min R:R Ratio** | 1.2:1 | 2.0:1 | 1.5:1 | 2.0:1 | 2.5:1 | Signal building (pre-execution) |
| **ATR Fallback %** | 0.012 | 0.020 | 0.015 | 0.025 | 0.018 | Autonomous level computation |

---

## 7. Debate Layer Signal Weights

### 7.1 BullTeam Evidence (Round 1) — 12 Factors

| Factor | Weight | Score Range | Notes |
|--------|--------|-------------|-------|
| Skill Consensus | 0.25 | -0.4 to 0.6 | Highest weight — aggregates 9 skills |
| Regime Alignment | 0.20 | -0.4 to 0.5 | Regime-adaptive scoring |
| RSI | 0.15 | -0.5 to 0.6 | Oversold = bullish opportunity |
| News Availability | 0.10 | 0 to 0.15 | No bad news = good news |
| Pattern Confluence | 0.10 | 0 to 0.3 | ≥2 patterns = strong signal |
| MACD Momentum | 0.10 | -0.2 to 0.3 | Histogram direction |
| Vector Memory | 0.10 | 0 to 0.15 | Similar past setups |
| **OBV Volume** | **0.12** | **-0.25 to 0.35** | **OBV direction > 0 = bullish volume confirmation** |
| **ADX Trend** | **0.10** | **-0.3 to 0.4** | **ADX > 25 + +DI > -DI = strong uptrend** |
| **CCI Momentum** | **0.08** | **-0.2 to 0.35** | **CCI > 100 = strong bullish, CCI < -100 = oversold bounce** |
| **Williams %R** | **0.08** | **-0.2 to 0.3** | **%R < -80 = oversold bounce opportunity** |
| **VWAP Flow** | **0.08** | **0 to 0.3** | **Price > VWAP = institutional buying pressure** |

> **Note:** New indicators (OBV, ADX, CCI, Williams %R, VWAP) carry ~31% of total evidence weight. They are supplementary — they reinforce or challenge existing signals rather than dominate.

### 7.2 BearTeam Evidence (Round 1) — 11 Factors

| Factor | Weight | Score Range | Notes |
|--------|--------|-------------|-------|
| Skill Consensus | 0.25 | -0.5 to 0.3 | Acknowledges but downplays bullish |
| Regime Risk | 0.25 | -0.6 to 0.2 | Bear regime = high risk |
| Portfolio Heat | 0.20 | -0.4 to 0 | >5% heat = strong negative |
| Consecutive Losses | 0.15 | -0.3 to 0 | ≥2 losses = fatigue |
| RSI Overbought | 0.10 | -0.25 to 0 | >65 = overbought |
| News Uncertainty | 0.10 | -0.15 to 0 | No news = risk |
| **OBV Volume** | **0.12** | **-0.35 to 0.15** | **OBV < -0.05 = bearish volume; Bull OBV acknowledged but downplayed** |
| **ADX Trend** | **0.10** | **-0.4 to 0.2** | **ADX > 25 + -DI > +DI = strong downtrend; Bull trend challenged** |
| **CCI Momentum** | **0.08** | **-0.35 to 0.2** | **CCI > 100 = overbought exhaustion; CCI < -100 = bearish momentum** |
| **Williams %R** | **0.08** | **-0.3 to 0.15** | **%R > -20 = overbought distribution; %R < -80 challenged** |
| **VWAP Flow** | **0.08** | **-0.3 to 0** | **Price < VWAP = institutional selling pressure** |

> **Dual-evidence pattern:** Both teams weigh in on extreme indicator values (e.g., BullTeam sees CCI > 100 as bullish momentum, BearTeam sees it as overbought exhaustion). This creates genuine debate rather than artificial consensus.

### 7.3 Adversarial Round (Round 2)

| Factor | Weight | Team | Notes |
|--------|--------|------|-------|
| Challenge opposing evidence | 0.30 | Both | Reduce opposing score by 50% |
| Unaddressed losses | 0.20 | Bear | Bull team ignores consecutive losses |
| Unaddressed heat | 0.15 | Bear | Bull team ignores portfolio heat |
| **OBV/price divergence** | **0.15** | **Bear** | **OBV bullish but MACD negative — volume/price divergence** |
| **ADX bear trend** | **0.15** | **Bear** | **ADX confirms downtrend (+DI < -DI) — unaddressed** |
| **VWAP institutional sell** | **0.15** | **Bear** | **Price below VWAP — institutional selling pressure** |
| Strong skill consensus | 0.20 | Bull | Bear team ignores strong bullish consensus |
| Oversold unaddressed | 0.15 | Bull | Bear team ignores RSI < 35 |
| **OBV bullish volume** | **0.15** | **Bull** | **Bear team ignores bullish OBV trend** |
| **ADX bull trend** | **0.15** | **Bull** | **ADX confirms uptrend (+DI > -DI) — unaddressed** |
| **CCI oversold bounce** | **0.15** | **Bull** | **CCI < -100 — deeply oversold, bounce likely** |
| **Williams %R oversold** | **0.15** | **Bull** | **%R < -80 — oversold, bounce signal ignored** |

### 7.4 Synthesis Round (Round 3)

| Factor | Weight | Notes |
|--------|--------|-------|
| Bull Proposal | 0.30 | Original bullish case |
| Bear Proposal | 0.30 | Original bearish case |
| Bull Counter | 0.20 | Adversarial impact |
| Bear Counter | 0.20 | Adversarial impact |
| Regime Tiebreaker | 0.10 | Breaks ties favoring trend |
| Adversarial Impact | 0.15 | Wounded side penalty |

### 7.5 Judge Veto Rules (Debate Quality Only)

> **Key change:** The Judge now ONLY evaluates debate quality. It does NOT re-run risk checks,
> regime checks, confluence checks, or R:R calculations. Those are enforced by the HardRulesGate (Layer 1).
> This separation of concerns prevents duplication and ensures each layer has a single responsibility.

| # | Rule | Condition | Action | Rationale |
|---|------|-----------|--------|-----------|
| 1 | Low Confidence | BUY + below regime-adaptive minimum | VETO → HOLD | Insufficient debate conviction |
| 2 | Evidence Contradiction | Bull/bear evidence gap < 0.10 | VETO → HOLD | Teams too evenly matched |
| 3 | Insufficient Signals | < 3 evidence factors | VETO → HOLD | Thin debate produces unreliable verdicts |

**Note:** The Judge reads `portfolio_heat` and `confluence` for informational reasoning strings only — these values do NOT influence veto decisions.

---

## 8. Position Sizing Multipliers

### 8.1 Adaptive Risk Multiplier

| Factor | Condition | Multiplier |
|--------|-----------|------------|
| **Confidence** | conf / 0.7 | 0.5–1.2 |
| **Consecutive Losses** | ≥ 3 | 0.5 |
| | ≥ 2 | 0.7 |
| | < 2 | 1.0 |
| **Portfolio Heat** | > 8% | 0.5 |
| | > 5% | 0.7 |
| | ≤ 5% | 1.0 |
| **Regime** | TrendingBull | 1.0 |
| | TrendingBear | 0.7 |
| | Ranging | 0.8 |
| | Volatile/LowLiq | 0.6 |

**Final multiplier**: `clamp(conf × loss × heat × regime, 0.3, 1.2)`
**Floor**: `max(rules.max_risk_per_trade × mult, 0.003)`

---

## 9. Learning & Evolution Systems (6)

| # | System | File | What It Does |
|---|--------|------|--------------|
| 1 | **MetaControl** | `meta_control.rs` | Rule adaptation + regime-specific skill weight evolution |
| 2 | **SelfEvolution** | `self_evolution.rs` | Walk-forward validation + regression detection |
| 3 | **WalkForwardRunner** | `walk_forward_runner.rs` | IS/OOS Sharpe validation (anti-overfitting) |
| 4 | **WeightTuner** | `weight_tuner.rs` | Attribution engine + symmetric reward/penalty |

### 9.1 MetaControl Regime-Specific Adjustments

| Regime | Skill Boost | Delta |
|--------|-------------|-------|
| TrendingBull | SentimentAnalyzer | +0.02 |
| TrendingBull | VolatilityCalculator | +0.01 |
| TrendingBull | MarketMetricsMeter | +0.02 |
| TrendingBull | **SupportResistance** | **+0.02** | **Breakout from S/R levels** |
| TrendingBull | **OrderFlow** | **+0.02** | **Volume confirms trend** |
| TrendingBear | SentimentAnalyzer | +0.03 |
| TrendingBear | CorrelationChecker | +0.02 |
| TrendingBear | RiskGuardian | +0.02 |
| TrendingBear | **FundingRate** | **+0.02** | **Crowded shorts = bullish** |
| TrendingBear | **Liquidity** | **+0.01** | **Avoid thin markets** |
| Ranging | PatternRetriever | +0.03 |
| Ranging | OnChainData | +0.02 |
| Ranging | RegimeDetector | +0.02 |
| Ranging | **SupportResistance** | **+0.03** | **Mean reversion at S/R** |
| Ranging | **VolumeProfile** | **+0.02** | **POC as magnet** |
| Volatile | VolatilityCalculator | +0.03 |
| Volatile | CorrelationChecker | +0.03 |
| Volatile | MarketMetricsMeter | +0.02 |
| Volatile | **OrderFlow** | **+0.02** | **Flow direction in chaos** |
| Volatile | **Liquidity** | **+0.02** | **Slippage risk critical** |
| LowLiquidity | NewsAnalyser | +0.03 |
| LowLiquidity | SentimentAnalyzer | +0.02 |
| LowLiquidity | OnChainData | +0.02 |
| LowLiquidity | **Liquidity** | **+0.03** | **Market quality paramount** |
| LowLiquidity | **FundingRate** | **+0.02** | **Funding spikes in thin markets** |

> **Normalization**: After adjustment, all weights are normalized to sum to ~1.0 (clamped 0.01–0.40 each).

---

## 10. 5-Layer Pipeline Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    HARD RULES GATE (Layer 1)                    │
│  Runs FIRST — before any agents. Priority-based blocking.      │
│  Critical/High → always block. Medium → block if no Higher.    │
│  Low → warnings only. If blocked, pipeline aborts immediately. │
│  No agents waste compute if hard rules fail.                   │
└─────────────────────────────────────────────────────────────────┘
                              ↓ (if passed)
┌─────────────────────────────────────────────────────────────────┐
│               IDENTIFIER + VERIFIER (Layer 2)                   │
│  Advisory only — gather data, produce COT entries.              │
│  Identifier: scanner, market_intel, pivots, confluence,        │
│              patterns, session_timer, red_folder (7 agents)     │
│  Verifier: risk_psych, risk_calc, reflector + guardian checks  │
│  These agents NEVER block the pipeline — the gate handled that.│
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│              WFA GATE (Layer 2.5) — Regime Consistency          │
│  On first trade of day: verify recent price action matches     │
│  declared regime. Lightweight spot-check, not full WFA.        │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│               DEBATE LAYER (Layer 3) — Advisory Only           │
│  Round 1: BullTeam (12 factors) vs BearTeam (11 factors + SELL) │
│  Round 2: Adversarial challenges (COUNTER/WEAKENED)            │
│  Round 3: Weighted synthesis with adversarial impact            │
│  6 agents provide evidence + confidence. No veto power.        │
│  Only the Judge (Layer 4) has decision-making power.           │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│           JUDGE / ADJUDICATOR (Layer 4) — Final Authority      │
│  Evaluates DEBATE QUALITY ONLY:                                │
│  - Confidence threshold (regime-adaptive)                      │
│  - Evidence contradiction (bull/bear gap too small)            │
│  - Insufficient signal count (< 3 factors)                     │
│  Does NOT re-run risk/regime/confluence/R:R checks.           │
│  Those are enforced by the HardRulesGate (Layer 1).            │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│              EXECUTION LAYER (Layer 5)                         │
│  Autonomous level computation (entry/SL/TP)                    │
│  Adaptive position sizing (confidence × loss × heat × regime)  │
│  Paper trade execution + outcome logging                       │
└─────────────────────────────────────────────────────────────────┘
```

### 10.1 Separation of Concerns

| Layer | Responsibility | Does NOT Do |
|-------|---------------|-------------|
| **HardRulesGate** | Enforce all hard rules with priority | Gather market data, compute signals |
| **Identifier** | Gather market intelligence, produce COT entries | Block on session/red_folder (gate handles) |
| **Verifier** | Risk analysis, position sizing, reflection | Block on drawdown/overtrading (gate handles) |
| **DebateLayer** | Multi-round adversarial evidence scoring | Have veto power (advisory only) |
| **Judge** | Evaluate debate quality, final adjudication | Re-run risk/regime/confluence checks |
| **Execution** | Autonomous levels, position sizing, trade | Make trading decisions (Judge decides) |

### 10.2 Information Ratio

```
IR = IC × √(Breadth)

Current Breadth:
  - 26 technical indicators
  - 27 candlestick patterns
  - 14 agent skills
  - 7 debate agents
  - 14 risk rules
  - 6 regime-adaptive thresholds
  - 5 market structure tools (S/R, volume profile, order flow, liquidity, funding)
  
Total independent signals: ~70+
Theoretical IR scaling: √70 ≈ 8.4x vs single-signal system
```

---

## 11. Priority-Based Conflict Resolution

> **Research-backed principle:** "The upper layer always overrides the lower layers.
> When rules conflict, the highest priority wins. Equal priority → most conservative action."

### 11.1 Priority Levels

| Priority | Behavior | Examples |
|----------|----------|----------|
| **Critical** | ALWAYS blocks — no override possible | Drawdown halt, session timing, red folder, trading disabled |
| **High** | ALWAYS blocks — no override possible | Heat limit, circuit breaker, max trades, cooldown |
| **Medium** | Blocks ONLY if no Critical/High rule has already been checked | Regime safety, confluence minimum |
| **Low** | WARNINGS ONLY — logged but never block | Max positions per symbol, max total positions |

### 11.2 Conflict Resolution Logic

```rust
// Pseudocode for priority-based blocking
let failed_rules = evaluate_all_rules();
let highest_blocking_priority = failed_rules
    .filter(|r| r.priority >= Medium)
    .map(|r| r.priority)
    .max();

// Critical/High always block. Medium blocks only if no Higher override.
// Low never blocks — warnings only.
let passed = highest_blocking_priority.is_none()
    || highest_blocking_priority == Some(Low);
```

### 11.3 Example Scenarios

| Scenario | Rules Failed | Result | Reason |
|----------|-------------|--------|--------|
| Drawdown 2.5% | Critical (drawdown) | **BLOCK** | Critical always blocks |
| Heat 12% + 3 positions/symbol | High (heat) + Low (positions) | **BLOCK** | High always blocks |
| Low confluence (0.45) | Medium (confluence) | **BLOCK** | Medium blocks if no Higher override |
| Only 4 positions on symbol | Low (positions) | **PASS** | Low = warning only |
| Session closed + low confluence | Critical (session) + Medium (confluence) | **BLOCK** | Critical always blocks |

### 11.4 Why This Matters

Without priority-based resolution:

---

## 12. SuperIntelligence Decision Layer (NEW)

> **Layer 3.5** — Between DebateLayer and Judge. Cross-validates every signal before it reaches the final decision.

### 12.1 Components

| Component | File | Purpose |
|-----------|------|---------|
| **CrossValidationEngine** | `super_intelligence.rs` | Validates each signal against ≥2 independent sources. 5 validation pairs. Detects conflicts and penalizes disagreements |
| **ConvictionStack** | `super_intelligence.rs` | 8-factor conviction: Directional + Confidence + Agreement + Memory + Risk + Pattern + Timeframe + Synthesis. Regime-adaptive coefficients |
| **DecisionTrace** | `super_intelligence.rs` | Every BUY/SELL emits ranked factor list showing WHY — factor name, weight, direction, contribution %, cross-validation status |
| **MemoryContext** | `super_intelligence.rs` | Structured memory recall with win rate, regret, and lessons from vector memory |

### 12.2 Conviction Factors (8)

| # | Factor | Source | Default Weight | Description |
|---|--------|--------|---------------|-------------|
| 1 | **Directional** | `score` from skill aggregation | 0.25 | Net directional score from skill consensus |
| 2 | **Confidence** | `confidence` from skill aggregation | 0.15 | Average confidence of all skill outputs |
| 3 | **Agreement** | Bull/Bear evidence count | 0.14 | Ratio of positive vs negative evidence factors |
| 4 | **Memory** | Vector memory recall | 0.14 | Win rate from similar historical setups |
| 5 | **Risk** | Portfolio heat + drawdown | 0.14 | Current risk level (inverted: low risk = high score) |
| 6 | **Pattern** | Candlestick pattern direction | 0.08 | Alignment of detected patterns with proposed action |
| 7 | **Timeframe** | Multi-TF confirmation | 0.06 | Cross-timeframe pattern confirmation (Strong=1.0, Moderate=0.7, Weak=0.4) |
| 8 | **Synthesis** | Cross-validation quality | 0.04 | Overall validation score from CrossValidationEngine |

### 12.3 Cross-Validation Pairs (5)

| Pair | Primary Source | Secondary Source | Rationale | Conflict Penalty |
|------|---------------|-----------------|-----------|-----------------|
| 1 | MarketMetricsMeter | RegimeDetector | Indicator ensemble vs trend direction | -0.15 |
| 2 | SupportResistance | VolumeProfile | Price levels vs volume distribution | -0.10 |
| 3 | OrderFlow | FundingRate | Buy/sell pressure vs market sentiment | -0.12 |
| 4 | SentimentAnalyzer | OnChainData | News sentiment vs accumulation | -0.10 |
| 5 | VolatilityCalculator | Liquidity | Volatility regime vs market quality | -0.08 |

### 12.4 Regime-Adaptive Conviction Coefficients

| Factor | TrendingBull | TrendingBear | Ranging | Volatile | LowLiquidity |
|--------|-------------|--------------|---------|----------|--------------|
| Directional | **0.30** | 0.15 | 0.20 | 0.12 | 0.10 |
| Confidence | 0.12 | 0.15 | **0.20** | 0.15 | 0.12 |
| Agreement | 0.10 | 0.15 | 0.15 | 0.12 | 0.10 |
| Memory | 0.10 | **0.20** | 0.15 | 0.12 | 0.10 |
| Risk | 0.10 | 0.15 | 0.10 | **0.25** | **0.35** |
| Pattern | **0.10** | 0.05 | **0.10** | 0.08 | 0.05 |
| Timeframe | 0.08 | 0.08 | 0.08 | 0.06 | 0.04 |
| Synthesis | 0.05 | 0.07 | 0.02 | **0.10** | **0.14** |
| **Total** | **1.00** | **1.00** | **1.00** | **1.00** | **1.00** |

### 12.5 Decision Trace Format

Every BUY/SELL decision includes this ranked output:

```
╔══ SUPERINTELLIGENCE DECISION TRACE ══╗
║ Action: BUY (conf 72.3%)
║ Conviction: 68.5% | Regime: TrendingBull (threshold 50%)
║ Risk: LOW RISK — heat 2.1%
╠══ Ranked Factors ══╣
║  #1. Regime/Trend [validated] (w=0.20) Bullish ↑ — 32.1% contribution
║  #2. Vol/Liquidity [validated] (w=0.08) Bullish ↑ — 28.5% contribution
║  #3. Levels/Structure [partial] (w=0.10) Bullish ↑ — 18.7% contribution
║ Conviction breakdown: Directional=28% | Confidence=22% | Agreement=15% | Memory=12% | ...
╚══════════════════════════════════════╝
```

---
- A **Low-priority** position-size preference would block the same as a **Critical** drawdown halt
- Adding hundreds of rules creates conflicts and overfitting
- No clear authority hierarchy → agents can override safety rules

With priority-based resolution:
- **Critical rules** (drawdown, session) are NEVER bypassed by any agent
- **Medium rules** (regime, confluence) are enforced but don't override higher authority
- **Low rules** (position limits) are informational — they inform but don't decide
- **Scalable**: adding new rules at any priority level never conflicts with existing ones

---

## Summary Counts

| Category | Count |
|----------|-------|
| Technical Indicators | 26 |
| Candlestick Patterns | 27 |
| Agent Skills | 14 |
| Market Structure Tools | 5 |
| Debate Agents | 7 |
| Risk & Discipline Rules | 14 |
| Regime-Adaptive Thresholds | 6 tables |
| Debate Signal Weights | 55+ factors |
| Position Sizing Rules | 4 multipliers |
| Learning Systems | 4 |
| SuperIntelligence Components | 4 |
| Cross-Validation Pairs | 5 |
| Conviction Factors | 8 |
| Regime-Adaptive Conviction Tables | 5 regimes |
| Pipeline Layers | 6 (Layer 3.5 added) |
| Priority Levels | 4 |
| **TOTAL INDICATORS + RULES + TOOLS** | **~130+** |
