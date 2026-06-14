# 🛡️ tredo Disciplined Core Specification

> **The Non-Negotiable Foundation** (Trading Real-time Edge Decision Optimisation) — Professional rules in Rust. The Terminal UI surfaces violations clearly.

```mermaid
flowchart TB
    INPUT[Market Event\nPrice Tick / Signal] --> GATE{Disciplined Core Gate}
    GATE -->|FAIL| REJECT[❌ Rejected\nNo trade taken]
    GATE -->|PASS| LLM{Proceed to\nLLM Reasoning?}
    LLM -->|Yes| AGENT[Agent Decision\nBUY / SELL / HOLD]
    LLM -->|No - Rules Sufficient| AUTO[Auto-Pass\nNo LLM needed]
    REJECT --> LOG[Log Violation]
    AGENT --> EXEC[Execute / Log]
    AUTO --> EXEC
```

---

## 📋 Core Categories

```mermaid
mindmap
  root((Disciplined Core))
    Technical Rules
      Daily Pivot Points
      Support & Resistance
      200 EMA Trend Filter
      Session Timing
    Confluence
      DXY Movement
      10Y Treasury Yields
      BTC Dominance
      Funding Rates
      On-Chain Flows
    Risk Management
      1% Max Risk Per Trade
      3% Max Daily Drawdown
      Position Sizing
      Loss Limit Halt
      Dynamic Accounting
    Psychology
      Red Folder Filter
      Consecutive Loss Reduction
      Overtrading Prevention
      Session Respect
    Entry Criteria
      Minimum Confluence Score
      Favorable Session
      No Red-Folder Event
      Risk Parameters Approved
```

---

## 1. 📐 Technical Rules

```mermaid
flowchart LR
    subgraph "Pivot Calculation"
        HIGH[Daily High] --> PIVOT
        LOW[Daily Low] --> PIVOT
        CLOSE[Previous Close] --> PIVOT
        PIVOT[Pivot Calculator] --> LEVELS[Pivot Levels\nR3 R2 R1 P S1 S2 S3]
    end
    
    subgraph "Trend Filter"
        EMA[200 EMA] --> TREND{Trend Direction}
        PRICE[Current Price] --> TREND
        TREND -->|Price > EMA| BULL[Bullish Bias]
        TREND -->|Price < EMA| BEAR[Bearish Bias]
    end
    
    subgraph "Session Check"
        TIME[Current Time IST] --> SESSION{Session Timer}
        SESSION -->|13:30-21:30 IST| LONDON[London Session ✓]
        SESSION -->|17:30-23:30 IST| NY[New York Session ✓]
        SESSION -->|Other| OUTSIDE[Outside Hours ✗]
    end
```

### Pivot Methods

| Method | Formula | Use Case |
|--------|---------|----------|
| **Classic** | `P = (H + L + C) / 3` | Default — general purpose |
| **Fibonacci** | `P = (H + L + C) / 3`, S/R via fib ratios | Trending markets |
| **Woodie** | `P = (H + L + 2C) / 4` | Momentum-driven |
| **Camarilla** | `P = (H + L + C) / 3`, tight S/R | Range-bound markets |

### Session Timing (IST)

| Session | Opens (IST) | Closes (IST) | Focus |
|---------|-------------|---------------|-------|
| London | 13:30 | 21:30 | European indices, FX majors |
| New York | 17:30 | 23:30 | US equities, NSE overlap |
| Asian | 05:30 | 13:30 | Crypto, JPY pairs |
| **Override** | Crypto (BTC/ETH/SOL) | **24/7** | Session check bypassed |

---

## 2. 🔗 Confluence Requirements

> Multiple factors must align before a trade is considered valid.

```mermaid
flowchart TB
    subgraph "Confluence Factors"
        DXY[DXY Direction] --> WEIGHT
        YIELD[10Y Treasury] --> WEIGHT
        BTC_DOM[BTC Dominance] --> WEIGHT
        FUNDING[Funding Rates] --> WEIGHT
        ONCHAIN[On-Chain Flows] --> WEIGHT
        PIVOT_ALIGN[Pivot Alignment] --> WEIGHT
        VOLUME[Volume Confirmation] --> WEIGHT
        PATTERN[Candlestick Pattern] --> WEIGHT
    end
    
    WEIGHT[Weighted Scorer] --> SCORE{Confluence Score}
    SCORE -->|≥ 0.7| HIGH[✅ Strong — Proceed]
    SCORE -->|0.5 - 0.7| MED[⚠️ Moderate — Caution]
    SCORE -->|< 0.5| LOW[❌ Weak — Reject]
```

### Factor Weights

| Factor | Weight | Data Source |
|--------|--------|-------------|
| Pivot S/R Alignment | 0.25 | Pivot Calculator |
| Trend Filter | 0.20 | 200 EMA |
| Candlestick Pattern | 0.15 | 15 Detectors (1m, 15m, 1h, 1d) |
| Volume Confirmation | 0.10 | OHLCV Volume |
| Kronos Forecast | 0.10 | Time-Series Prediction |
| On-Chain Flows | 0.10 | Crypto-specific |
| News Sentiment | 0.10 | RSS Feeds / Summarized |

---

## 3. ⚠️ Risk Management (Hard Rules)

```
These rules are NON-NEGOTIABLE — they CANNOT be overridden by any agent.
```

```mermaid
flowchart TD
    TRADE[Trade Request] --> R1{Max Risk\n≤ 1% of Equity?}
    R1 -->|No| REJECT1[❌ Reject: Over-Leveraged]
    R1 -->|Yes| R2{Max Daily DD\n≤ 3%?}
    R2 -->|No| HALT[🛑 HALT ALL TRADING]
    R2 -->|Yes| R3{Consecutive Losses\n≤ Max?}
    R3 -->|No| R4{Reduce Size?\n50% Penalty Active}
    R3 -->|Yes| R5{Portfolio Heat\n≤ 15%?}
    R4 --> R5
    R5 -->|No| REJECT2[❌ Reject: Portfolio Saturated]
    R5 -->|Yes| PASS[✅ Trade Approved]
```

| Rule | Limit | Enforcement | Behavior on Violation |
|------|-------|-------------|----------------------|
| Max Risk per Trade | 1% of equity | Per-trade sizing | Position size capped |
| Max Daily Drawdown | 3% of equity | Continuous monitor | 🛑 Halt all trading |
| Max Consecutive Losses | Configurable (default 3) | Per-trade counter | 50% size penalty, halt at limit |
| Max Portfolio Heat | 15% of equity | Continuous monitor | No new positions |
| Max Daily Trades | Configurable (default 10) | Daily counter | Block new trades |
| Min Confidence | Based on trading mode | Per-signal | HOLD if below threshold |

### Dynamic Accounting

```rust
// LONG position: P&L = (current_price - entry_price) * quantity
// SHORT position: P&L = (entry_price - current_price) * quantity
//
// Cash balance decreases on entry, increases on exit + P&L
// Equity = cash + sum of unrealized P&L across all open positions

// After SHORT sale:
// Cash += entry_price * quantity (proceeds from sale)
// Position liability = quantity shares at current price

// After SHORT close:
// Cash -= exit_price * quantity (buy back)
// P&L = entry_price * quantity - exit_price * quantity
```

---

## 4. 🧠 Psychology & Discipline

```mermaid
flowchart LR
    subgraph "Red Folder Check"
        EVENT[Economic Event] --> RF{High Impact?}
        RF -->|Yes| BLOCK[🚫 Block: 30 mins before/after]
        RF -->|No| ALLOW[✅ Allow Trading]
    end
    
    subgraph "Consecutive Loss Adjustment"
        LOSSES[Trade Counter] --> CL{≥ 2 Losses?}
        CL -->|Yes| REDUCE[📉 Reduce Position Size × 0.5]
        CL -->|No| NORMAL[📊 Normal Position Size]
    end
    
    subgraph "Overtrading Prevention"
        COUNT[Daily Trade Count] --> OT{≥ Max Trades?}
        OT -->|Yes| STOP[🛑 Stop Trading for Day]
        OT -->|No| CONTINUE[✅ Continue Trading]
    end
```

| Psych Rule | Trigger | Response | Duration |
|------------|---------|----------|----------|
| Red Folder Filter | High-impact economic event | Block trades ±30 minutes | 1 hour per event |
| Consecutive Loss Reduction | 2+ losses in a row | Reduce position size by 50% | Until a winning trade |
| Consecutive Loss Halt | Losses exceed max threshold | Halt all trading | End of day |
| Overtrading Prevention | Trades exceed max daily count | Block new trades | End of day |
| Mode-Based Confidence | Trading mode changed | Adjust min confidence threshold | Until mode changes |

---

## 5. ✅ Entry Criteria

```mermaid
flowchart TB
    START[🔄 New Signal Candidate] --> CHECK1{Minimum Confluence\n≥ 0.5?}
    CHECK1 -->|No| FAIL1[❌ Fail: Insufficient confluence]
    CHECK1 -->|Yes| CHECK2{Favorable Session?\nOR Crypto?}
    CHECK2 -->|No| FAIL2[❌ Fail: Outside trading hours]
    CHECK2 -->|Yes| CHECK3{Red Folder Event?\nWithin ±30 min?}
    CHECK3 -->|Yes| FAIL3[❌ Fail: High-impact news imminent]
    CHECK3 -->|No| CHECK4{Risk Parameters\nAll Passed?}
    CHECK4 -->|No| FAIL4[❌ Fail: Risk violation]
    CHECK4 -->|Yes| CHECK5{Daily Loss Limit\nNot Hit?}
    CHECK5 -->|No| FAIL5[❌ Fail: Loss limit exceeded]
    CHECK5 -->|Yes| PASS[✅ ALL CHECKS PASSED\nProceed to LLM / Execution]
```

### Checklist for a Valid Trade

```
[✅] Confluence Score ≥ Minimum Threshold
[✅] Within Valid Trading Session (London/NY) OR Crypto
[✅] No Red-Folder High-Impact Event ±30 minutes
[✅] Max Risk ≤ 1% of Account Equity
[✅] Max Daily Drawdown ≤ 3%
[✅] Not in Consecutive Loss Halt State
[✅] Portfolio Heat ≤ 15%
[✅] Trades Today ≤ Max Daily Trades
[✅] Position Sizing Correct (based on stop distance)
[✅] Entry Criteria Met (direction, SL/TP, confidence)
```

---

## 🏗️ Implementation Principles

```rust
// Written in Rust for speed and reliability
// Loaded at startup — zero runtime overhead
// Sub-Agents can make many decisions using only this core
// Main Agents consult it before using LLM

pub fn validate_trade_setup(context: &MarketContext, rules: &DisciplineRules) -> DisciplineResult {
    let mut all_reasons = Vec::new();
    let mut overall_passed = true;

    // 1. Session check (with crypto bypass)
    if !is_crypto && !is_in_trading_session(context.timestamp, rules) {
        all_reasons.push("Outside allowed trading sessions".to_string());
        overall_passed = false;
    }

    // 2. Confluence check
    let confluence = calculate_confluence_score(context, &pivots);
    if confluence < rules.min_confluence {
        all_reasons.push(format!("Confluence too low: {:.2}", confluence));
        overall_passed = false;
    }

    // 3. Risk checks
    if context.daily_pnl.abs() >= rules.max_daily_drawdown * context.total_equity {
        all_reasons.push("Daily drawdown limit reached".to_string());
        overall_passed = false;
    }

    // ... additional checks

    DisciplineResult { passed: overall_passed, reasons: all_reasons }
}
```

---

## 🎯 Goal

> Create agents that behave like **experienced, disciplined traders** who follow rules first and use intelligence second.

### Skills + Rules + Trained Memory Integration

The Disciplined Core is the **"what to do / what not to do"** layer (hard, fast, in Rust, non-overridable by LLM).

It is complemented by:
- **Skills** (`AgentSkill` trait): the "how" (pluggable analyzers and tools that agents execute to gather richer signals before rules are even consulted).
- **Trained memory adjustments**: `apply_trained_memory_to_rules(rules, recall)` in this module dynamically strengthens the rules (e.g. raises `min_confluence_score` or lowers `max_risk_per_trade`) when hierarchical recall surfaces past regret or cautionary lessons on similar setups. This is called from `StrategyDecisionAgent` (and can be used anywhere) right before debate/LLM.

Result: rules evolve safely with real experience ("trained intelligence") while remaining the single source of truth for safety. Sub-agents and main agents stay aware via `recall_trained_memory`. Full details and the exact philosophy ("skills tell how, rules tell what/not, agents already know their roles") are in `tredo-core/src/skills.rs` (header), `disciplined_core.rs`, and the agent files that call them.
