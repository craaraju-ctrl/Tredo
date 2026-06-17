# 🧠 tredo Agent Design

**Trading Real-time Edge Decision Optimisation** — Full Terminal UI + Two-Tier Hierarchical Architecture.

Main agents orchestrate with LLM and structured debate. Sub-agents are fast, deterministic, and LLM-free for safety and performance.

---

## 🏗️ Agent Hierarchy & Logical Groups (Tredo)

tredo structures its agents into the **`Tredo`** four-group orchestrator (Identifier / Verifier / Executer / Guardian). The beautiful Terminal UI (`tredo tui`) is the primary way to watch and control the live system.

The **Runtime Engine** (`tredo-runtime`) wraps this hierarchy into an event-driven, multi-mode system with a world model, policy cache, and broker plugin system.

```mermaid
graph TB
    subgraph "Tredo Logical Hierarchy"
        subgraph "1. Identifier [Market & Setup]"
            WS[WatchlistScannerAgent]
            MI[MarketIntelligenceAgent]
            PC[PivotCalculatorAgent]
            CS[ConfluenceScorerAgent]
            PR[PatternRetrieverAgent]
            ST[SessionTimerAgent]
            RFC[RedFolderCheckerAgent]
        end

        subgraph "2. Verifier [Pre-Trade Validation]"
            RP[RiskPsychologyAgent]
            RC[RiskCalculatorAgent]
            REF[ReflectorAgent]
        end

        subgraph "3. Executer [Execution]"
            SD[StrategyDecisionAgent]
            PM[PortfolioManagerAgent]
            EXEC[ExecutionCoordinatorAgent]
        end

        subgraph "4. Guardian [Account Safeguards]"
            DM[DrawdownMonitorAgent]
            OP[OvertradingPreventerAgent]
            OL[OutcomeLoggerAgent]
        end
    end

    subgraph "Runtime Layer (tredo-runtime)"
        RT[RuntimeEngine]
        WM[WorldModelEngine]
        PCACHE[PolicyCache]
        AL[ActiveLearner]
        INTRO[Introspector]
        GM[GoalManager]
        BR[BrokerPluginManager]
    end

    RT -->|orchestrates| TREDO
    RT -->|updates| WM
    RT -->|queries| PCACHE
    RT -->|probes| AL
    RT -->|introspects| INTRO
    RT -->|manages goals| GM
    RT -->|dispatches trades| BR
```

---

## 🧩 Main Agent Responsibilities

```mermaid
flowchart LR
    subgraph "Perception Layer"
        MI[MarketIntelligence]
    end
    subgraph "Reasoning Layer"
        SD[StrategyDecision]
        RP[RiskPsychology]
        REF[Reflector]
    end
    subgraph "Execution Layer"
        PM[PortfolioManager]
        EXEC[ExecutionCoordinator]
    end
    subgraph "Meta Layer"
        MC[MetaControl]
    end

    MI -->|Confluence Score| SD
    MI -->|Market Context| RP
    SD -->|TradeSignal| PM
    SD -->|TradeSignal| EXEC
    RP -->|RiskAnalysis| SD
    PM -->|Position Update| EXEC
    REF -->|Lessons| MC
    MC -->|Rule Adjustment| RP
```

| Agent | Role | LLM Usage | Key Responsibilities |
|-------|------|-----------|---------------------|
| **MarketIntelligence** 🔍 | Data fusion & regime detection | Low–Medium | Confluence scoring, pivot calculation, Kronos forecast, candlestick pattern detection (15 patterns across 4 timeframes) |
| **StrategyDecision** 🤖 | Trade signal generation | Medium | LLM-driven BUY/SELL/HOLD with enriched context (Kronos forecast, calendar events, vector memory, news, multi-TF patterns) |
| **RiskPsychology** 🧠 | Risk management & discipline | Low | Position sizing, drawdown control, portfolio heat, psychology warnings (revenge trading, overtrading) |
| **Reflector** 💡 | Post-trade review & learning | Medium | Outcome analysis, violated assumptions, regret scoring, lesson extraction |
| **PortfolioManager** 📊 | Overall exposure & accounting | Low | LONG/SHORT accounting, cash balance management, position correlation |
| **ExecutionCoordinator** ⚡ | Final safety & paper execution | Very Low | Slippage check, liquidity check, kill-switch, SL/TP auto-exit |
| **MetaControl** 🔄 | Rule self-adjustment | Medium | Weekly review of high-regret episodes, LLM-proposed rule changes |

---

## ⚙️ Sub-Agent Specifications

All Sub-Agents are **deterministic, pure-logic** computations with no LLM dependency. They execute in milliseconds and form the backbone of the system's reliability.

### Technical Sub-Agents

| Sub-Agent | Input | Output | Algorithm |
|-----------|-------|--------|-----------|
| **PivotCalculator** | High, Low, Close | `PivotLevels { pivot, r1, r2, r3, s1, s2, s3 }` | Classic / Fibonacci / Woodie / Camarilla |
| **ConfluenceScorer** | MarketContext + PivotLevels | Score (0.0–1.0) | Multi-factor weighted sum: trend alignment, S/R proximity, volume confirmation, volatility |
| **SessionTimer** | Timestamp | `SessionInfo { open, name, time_remaining }` | IST-aware: London (13:30 IST) + NY (17:30 IST) |

### Risk Sub-Agents

| Sub-Agent | Input | Output | Algorithm |
|-----------|-------|--------|-----------|
| **PositionSizer** | Equity, Risk%, Entry, Stop | Position size (units) | `equity × risk% / |entry - stop|` |
| **DrawdownMonitor** | Daily P&L, Equity | Max drawdown %, HALT if exceeded | Track峰值 → trough, compare to max_daily_drawdown (3%) |

### Psychology Sub-Agents

| Sub-Agent | Input | Output | Algorithm |
|-----------|-------|--------|-----------|
| **RedFolderChecker** | Calendar events, Symbol | `bool` (blocked / allowed) | Matches symbol against high-impact events, synchronized to IST |
| **OvertradingPreventer** | Trade count, Max trades per day | `bool` (allowed / blocked) | `trade_count >= max_daily_trades → BLOCK` |

### Memory Sub-Agents

| Sub-Agent | Input | Output | Algorithm |
|-----------|-------|--------|-----------|
| **OutcomeLogger** | TradeSignal, Outcome | Stored episode | Writes structured `TradingEpisode` to redb + LanceDB |
| **PatternRetriever** | Current MarketContext | `Vec<PatternMatch>` | Searches historical episodes, ranks by similarity score |

---

## 🔄 Pipeline Phase Flow (5-Layer Architecture)

```mermaid
flowchart TD
    START[Start Pipeline] --> P0{Phase 0:\nOpen Position?}
    P0 -->|Yes| SKIP[Skip — Position Exists]
    P0 -->|No| L1[Layer 1:\nHardRulesGate\nPriority-based blocking]
    
    L1 --> L1C{All Critical/High\nrules pass?}
    L1C -->|FAIL| L1F[Return: Gate Blocked\n(priority reason)]
    L1C -->|PASS| L2[Layer 2:\nIdentifier + Verifier\nAdvisory — data gathering only]
    
    L2 --> L2A[Scanner + Market Intel]
    L2 --> L2B[Pivots + Confluence]
    L2 --> L2C[Patterns + Session]
    L2 --> L2D[Risk Psychology]
    L2 --> L2E[Risk Calculator + Reflector]
    
    L2 --> WFA{WFA Gate:\nRegime Consistency}
    WFA -->|FAIL| WFAF[Return: Regime Inconsistent]
    WFA -->|PASS| L3[Layer 3:\nDebateLayer\nAdvisory — no veto power]
    
    L3 --> R1[Round 1: BullTeam 12 factors\nvs BearTeam 11 factors]
    R1 --> R2[Round 2: Adversarial\nchallenges with new indicators]
    R2 --> R3[Round 3: Synthesis\nweighted verdict]
    
    R3 --> L4[Layer 4:\nJudge/Adjudicator\nDebate quality evaluation only]
    L4 --> L4C{Confidence OK?\nEvidence consistent?}
    L4C -->|VETO| L4F[Return: HOLD\n(debate quality)]
    L4C -->|APPROVE| L5[Layer 5:\nExecution\nAutonomous levels + sizing]
    
    L5 --> L5A[Paper Trade Fill]
    L5 --> L5B[Portfolio Update]
    L5 --> L5C[COT Entry]
    L5 --> DONE[Done: Trade Executed]
```

---

## 🎭 Agent Personas

Each Main Agent is designed with a distinct **trading personality**:

| Agent | Persona | Voice |
|-------|---------|-------|
| **MarketIntelligence** | The Analyst | "Confluence is 0.72, pivot at 24,500 is holding. R1 at 24,620 would be my first target." |
| **StrategyDecision** | The Trader | "I'm seeing bullish engulfing on 1m with 75% strength. Kronos confirms upward drift. I'll take the long." |
| **RiskPsychology** | The Risk Officer | "Portfolio heat at 12%, DD at 1.2%, 3 consecutive losses. I'm recommending size reduction." |
| **Reflector** | The Mentor | "You entered during low confluence (0.35). Wait for confirmation next time. Regret score: 0.7." |
| **MetaControl** | The Coach | "Reviewing 12 episodes: 4 high-regret. Pattern: entering before FOMC. Adjusting max_risk_per_trade to 0.8%." |

---

## 💡 Design Rules

1. **Sub-Agents must be deterministic and fast** — they are the foundation of reliability
2. **Main Agents act as coordinators** — they delegate to Sub-Agents and synthesize results
3. **Most decisions should be resolved by Sub-Agents + Disciplined Core** — without invoking LLM
4. **LLM is only used when uncertainty is high or synthesis is complex** — it's a scarce resource
5. **Every decision must be auditable** — chain-of-thought entries capture the full reasoning path
6. **Agents should feel like specialized trading professionals** — not generic AI assistants

---

## 🛠️ Strong Skills + Rules + Roles + Trained Memory + Policy Cache (Explicit Contract)

Agents and sub-agents **already know what to do** (their Tredo role and responsibilities in the five-layer pipeline).

### 5-Layer Architecture

```text
Layer 1: HardRulesGate — ALL hard rules with priority-based blocking
  Critical/High → always block. Medium → block if no Higher override. Low → warnings only.
  Runs FIRST — no agents waste compute if hard rules fail.

Layer 2: Identifier + Verifier — Advisory only
  Market intelligence gathering, risk analysis, confluence, pivots, patterns.
  These agents NEVER block the pipeline — the gate handled that.

Layer 3: DebateLayer — Advisory only (no veto power)
  Round 1: BullTeam (12 factors) vs BearTeam (11 factors + SELL proposal)
  Round 2: Adversarial challenges (OBV, ADX, CCI, Williams %R, VWAP)
  Round 3: Weighted synthesis with adversarial impact

Layer 4: Judge/Adjudicator — Final authority (debate quality ONLY)
  Evaluates: confidence threshold, evidence contradiction, signal count.
  Does NOT re-run risk/regime/confluence checks — those are in Layer 1.

Layer 5: Execution — Autonomous levels + adaptive sizing
```

### Component Roles

- **Skills** (`tredo-core/src/skills.rs` — the `AgentSkill` trait + `TrainedMemorySkill` + `SkillWrapper`) tell them **how to do** things. Pluggable and executable: SentimentAnalyzer, VolatilityCalculator, regime detection, patterns, and the key `TrainedMemorySkill`. Agents/sub-agents can hold or receive `Vec<Box<dyn AgentSkill>>` and call `execute`.
- **Rules** (`tredo-core/src/disciplined_core.rs`) tell **what to do and what not to do** (`DisciplineRules`, `validate_trade_setup`, `check_risk_limits`, etc.). These are now memory-aware via `apply_trained_memory_to_rules` (past regret/lessons can tighten confluence or risk limits dynamically).
- **Hierarchical Trained Memory** (`SharedState::recall_trained_memory`) is how agents and sub-agents **understand exactly what they were doing** before: combines fast local vector RAG (recent trained episodes with regret) + long-term `AgentMemoryClient` (shared "trained intelligence" lessons). Used in StrategyDecision, every debate participant, MarketIntelligence, Reflector, MetaControl, and even subs. Results are injected into COT, reasoning, and rule adjustments.
- **Policy Cache** (`tredo-runtime/src/policy_cache.rs`) is the **learned trading memory** that records (market features → action → outcome) tuples and short-circuits expensive Ollama debate when the system has seen a similar setup before.
- **World Model** (`tredo-runtime/src/world_model.rs`) maintains persistent beliefs about symbols, cross-symbol correlations, macro state, and active hypotheses.

See also the Core Philosophy section in the root README and the dedicated sections in Research.md / Build.md.