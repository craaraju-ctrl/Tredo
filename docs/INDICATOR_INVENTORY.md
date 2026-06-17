# TRED Indicator & Rule Inventory

> Single source of truth for every indicator, rule, threshold, and signal weight in the TRED autonomous trading system.
> Last updated: June 2026

---

## Table of Contents

1. [Technical Indicators (14)](#1-technical-indicators-14)
2. [Candlestick Patterns (16)](#2-candlestick-patterns-16)
3. [Agent Skills (9)](#3-agent-skills-9)
4. [Debate Layer Agents (7)](#4-debate-layer-agents-7)
5. [Risk & Discipline Rules (14)](#5-risk--discipline-rules-14)
6. [Regime-Adaptive Thresholds](#6-regime-adaptive-thresholds)
7. [Debate Layer Signal Weights](#7-debate-layer-signal-weights)
8. [Position Sizing Multipliers](#8-position-sizing-multipliers)
9. [Learning & Evolution Systems (4)](#9-learning--evolution-systems-4)
10. [5-Layer Pipeline Architecture](#10-5-layer-pipeline-architecture)
11. [Priority-Based Conflict Resolution](#11-priority-based-conflict-resolution)

---

## 1. Technical Indicators (14)

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

---

## 2. Candlestick Patterns (16)

| # | Pattern | Function | Bars Required | Type |
|---|---------|----------|---------------|------|
| 1 | Doji | `detect_doji()` | 1 | Neutral |
| 2 | Hammer | `detect_hammer()` | 1 | Bullish reversal |
| 3 | Shooting Star | `detect_shooting_star()` | 1 | Bearish reversal |
| 4 | Marubozu | `detect_marubozu()` | 1 | Continuation |
| 5 | Spinning Top | `detect_spinning_top()` | 1 | Neutral |
| 6 | Bullish Engulfing | `detect_bullish_engulfing()` | 2 | Bullish reversal |
| 7 | Bearish Engulfing | `detect_bearish_engulfing()` | 2 | Bearish reversal |
| 8 | Bullish Harami | `detect_bullish_harami()` | 2 | Bullish reversal |
| 9 | Bearish Harami | `detect_bearish_harami()` | 2 | Bearish reversal |
| 10 | Piercing Line | `detect_piercing_line()` | 2 | Bullish reversal |
| 11 | Dark Cloud Cover | `detect_dark_cloud_cover()` | 2 | Bearish reversal |
| 12 | Morning Star | `detect_morning_star()` | 3 | Bullish reversal |
| 13 | Evening Star | `detect_evening_star()` | 3 | Bearish reversal |
| 14 | Three White Soldiers | `detect_three_white_soldiers()` | 3 | Bullish continuation |
| 15 | Three Black Crows | `detect_three_black_crows()` | 3 | Bearish continuation |
| 16 | Multi-TF Confirmation | `detect_patterns_multi_tf()` | Cross-timeframe | Combined |

---

## 3. Agent Skills (9)

| # | Skill | File | Default Weight | Domain |
|---|-------|------|----------------|--------|
| 1 | SentimentAnalyzer | `sentiment_analyzer.rs` | 0.14 | News/market sentiment scoring |
| 2 | VolatilityCalculator | `volatility_calculator.rs` | 0.10 | Vol measurement + regime detection |
| 3 | RegimeDetector | `regime_detector.rs` | 0.12 | Market regime classification |
| 4 | CorrelationChecker | `correlation_checker.rs` | 0.06 | Cross-symbol correlation risk |
| 5 | OnChainData | `on_chain_data.rs` | 0.08 | Blockchain mempool/hashrate/volume |
| 6 | TrainedMemorySkill | `skills.rs` | 0.12 | Vector memory recall |
| 7 | PatternRetriever | `pattern_retriever.rs` | 0.10 | Historical pattern matching |
| 8 | NewsAnalyser | `news_analyser.rs` | 0.14 | Multi-source news sentiment |
| 9 | MarketMetricsMeter | `market_metrics_meter.rs` | 0.14 | Rich indicator bundle |

> **Total weights: ~1.0** — Adjusted by MetaControl based on regime-specific accuracy.

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

## 9. Learning & Evolution Systems (4)

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
| TrendingBear | SentimentAnalyzer | +0.03 |
| TrendingBear | CorrelationChecker | +0.02 |
| TrendingBear | RiskGuardian | +0.02 |
| Ranging | PatternRetriever | +0.03 |
| Ranging | OnChainData | +0.02 |
| Ranging | RegimeDetector | +0.02 |
| Volatile | VolatilityCalculator | +0.03 |
| Volatile | CorrelationChecker | +0.03 |
| Volatile | MarketMetricsMeter | +0.02 |
| LowLiquidity | NewsAnalyser | +0.03 |
| LowLiquidity | SentimentAnalyzer | +0.02 |
| LowLiquidity | OnChainData | +0.02 |

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
  - 14 technical indicators
  - 16 candlestick patterns
  - 9 agent skills
  - 7 debate agents
  - 14 risk rules
  - 6 regime-adaptive thresholds
  
Total independent signals: ~50+
Theoretical IR scaling: √50 ≈ 7x vs single-signal system
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
| Technical Indicators | 14 |
| Candlestick Patterns | 16 |
| Agent Skills | 9 |
| Debate Agents | 7 |
| Risk & Discipline Rules | 14 |
| Regime-Adaptive Thresholds | 6 tables |
| Debate Signal Weights | 55+ factors |
| Position Sizing Rules | 4 multipliers |
| Learning Systems | 4 |
| Pipeline Layers | 5 |
| Priority Levels | 4 |
| **TOTAL INDICATORS + RULES** | **~110** |
