# Remaining Implementation, Validation & Build Blueprint for tredo (TREDO) — 2026-06-14

**Context**: After full validation (real-time paper crypto with live Binance on BTC/ETH/SOL), debate wiring, auto-reflect, meta rule adaptation, COT, and skills hardening (RegimeDetector, CorrelationChecker, OnChainData now full AgentSkill + wired), the core "intact" self-evolving loop is working and observable.

However, the system is **not yet production-complete** for ambitious autonomous agentic trading. This document researches the still-pending items from Build.md roadmap, code stubs/TODOs, and prior audits. It provides:
- Research-backed insights (2026 sources, arXiv-style papers, GitHub repos, crates).
- Concrete **Rust-first** implementation plans.
- **Validation** focused on real-time paper trading with crypto (no simulation — live Binance feeds + autonomous decisions + paper fills).
- Build steps and testing commands.
- Prioritized order aligned with Build.md "Recommended order for intact system".

All changes must respect **Rust-high priority** (Kronos Python is the only justified gap).

## 1. Identified Still-Pending Items (Prioritized)

From Build.md §8 roadmap table + §10 validation + code greps (paper_engine stubs, vector_memory, llm TODOs, orchestrator TODO, .bak files, correlation notes):

**High Priority (must for "autonomous agentic" + self-evo credibility)**:
1. **Extended self-evolution validation + observable compounding** (Build.md: "Reflection + Meta — Partial wiring"; validation note: "ready for longer runs"; "Debate rarely fires trade in <5m" — needs sequences).
2. **Realistic paper execution (LOB/slippage) + broker adapters** (Build.md: "Execution (paper/live) — Paper stub"; "Backtester is placeholder"; live broker stubs in paper_engine.rs deferred until "paper hands-off is perfect").
3. **Full LanceDB integration** (Build.md: "Memory (redb + vector) — vector prototype"; feature exists but arrow conflicts + not default/used at scale; JSON fallback is current).
4. **Richer skills/tools + meta-adaptation of skills** (Build.md Skills section post-audit: "outputs still mostly side-effect"; "Self-evolution currently targets rules only"; "Expand library"; AgentOutput limited).
5. **Real-time data feeds (Binance WS vs REST polling)** (Build.md: "Data feeds — Mixed (Binance WS, Yahoo)"; current is REST in loops.rs; architecture promises WS).

**Medium**:
6. Clean duplication (tredo-agents deprecated shim).
7. Watchlist/env vs redb consistency + Ollama reliability (405s seen).
8. Production hardening (Docker, OTEL/tracing, error handling).

**Low/Justified**:
- Replace Kronos (explicit exception).
- Tauri (secondary).

Current state per Build.md §10: "The intact self-evolving agentic system is validated end-to-end with real-time crypto paper data. Ready for extended paper validation runs before any live broker."

## 2. Research on Pending Items (Key Insights 2026)

### 2.1 Extended Self-Evolution (Regret → Reflection → Meta Compounding)
- **Research Sources**: 
  - arXiv-style: "Trace2Policy: From Expert Behavior Traces to Self-Evolving Decision Agents" (error clusters → rule patching via EISR, regression-gated, compile to deterministic code). Self-rewarding contrastive distillation, RAGEN (verifiable rewards + self-bootstrapping verifiers for agents).
  - Trading-specific: TradingAgents (multi-agent LLM framework with specialized roles + collaboration); TradingGPT (layered memory + characters); older RL papers on meta-RL for trading (MAML, RL2 for fast adaptation); regret analysis in MARL.
  - Agent patterns: Self-Play Fine-Tuning, REvolve (reward evolution with human/LLM feedback), Evo-Memory benchmarks for test-time learning.
- **Key Insights for tredo**: Use "closed-loop" like Trace2Policy: Cluster high-regret episodes by cause (e.g., "high vol + low corr"), patch rules or skill params, gate with backtests on recent episodes. Combine with trained memory (episodes as "traces"). For trading, prioritize regret-based over pure reward (matches current OutcomeProcessor).
- **Validation Focus**: Real paper crypto — force sequences of regret (tight SL on BTC/ETH during volatile periods or use memecoins), track metrics over 50+ episodes: avg regret trend, rule value history (max_risk), win-rate improvement, # RULE_ADAPT events. Compare "before/after meta" runs. Use `./tredo validate --long --induce` .
- **Risks**: Overfitting to recent regimes (mitigate with regime_detector + multi-timeframe).

### 2.2 Realistic Paper + LOB + Broker Adapters (Crypto/Binance Focus)
- **Research Sources**:
  - Binance: Official docs for @depth / @depth@100ms streams + "How to manage a local order book correctly" (snapshot REST + apply diffs, drop old events by updateId).
  - GitHub examples: tesser (Rust exchange connectors: tesser-paper for deterministic sim + tesser-binance WS/REST); genesis2025 (paper trading engine with order book features like OFI/OBI, simulated fills).
  - Rust crates: binance-rs / binance-spot / binance-futures-rs (WS with depth/trade streams, typed); tokio-tungstenite examples for raw Binance WS.
- **Key Insights**: For paper, maintain local order book from live depth WS + REST snapshot. Simulate realistic fills (match against book levels, volume impact for slippage). Current market-price + fixed % slippage is too naive. Broker adapters: Thin trait over PaperBroker vs real (Binance uses API keys for signed orders; paper ignores).
- **Validation**: Real-time paper crypto — subscribe depth for BTCUSDT, feed into PaperEngine, place "paper" orders, observe more variable fills vs current. Measure P&L realism vs simple sim. For broker: Start with Binance futures/spot adapter stub (gated).
- **Implementation Notes**: Extend PaperEngine with `LocalOrderBook` struct + `apply_depth_update`. Use in fast loop.

### 2.3 Full LanceDB (Production Vector Memory/RAG)
- **Research Sources**:
  - Official: lancedb Rust docs (connect, create_table with arrow RecordBatch, query with vector search + filters). Integrations: rig-lancedb (with LLM embeddings), arrow-rs for schema.
  - Examples: "Scale Up Your RAG: Rust-Powered Indexing with LanceDB + Candle" (Arrow schemas, FixedSizeList for vectors, metadata); rig-lancedb tutorial (EmbeddingsBuilder + LanceDB store); Conf42 talk on LanceDB internals (Rust, Arrow, IVF_PQ index, in-process, no server).
  - Conflict fixes: Pin arrow versions in Cargo.toml (e.g., arrow-array compatible with chrono features); feature-gate lancedb behind optional; use serverless/embedded mode.
- **Key Insights**: Perfect for tredo (embedded, fast, Arrow-native for trading data + embeddings from Ollama). Current JSON VectorMemory (cosine) is brute-force prototype. Replace with Lance table: columns for embedding (vector), symbol, regret_score, lesson, timestamp, episode_json. Query with filters (e.g., recent high-regret) + vector search.
- **Validation**: Real paper runs — compare recall quality/speed (trained memory injection in MI/debate) before/after. Measure RAG hit rate on episodes that improved outcomes.
- **Build**: Add to Cargo features; handle arrow pin (e.g., [dependencies] arrow = { version = "51", ... } with lancedb). Migrate vector_memory.rs.

### 2.4 Richer Skills + Meta on Skills + Observability
- **Research Sources**: Toolformer / GPT4Tools (LLMs self-teach tool use); self-evolving agents via feedback on tool effectiveness; meta-learning for tool selection in agent papers (e.g., in MARL/RLVR "self-bootstrapping").
- **Key Insights**: Current skills (direct calls + partial trait vec) contribute via side-effects/manual extra_score. Extend AgentOutput with SkillResult variant for structured return (score + note). Collect in MI/debate for richer prompts/COT. For meta: Analyze episode outcomes vs skill scores (e.g., "high onchain score but high regret? → lower onchain weight").
- **Validation**: Paper crypto runs — log per-decision skill contributions + correlate with post-trade regret. Trigger meta skill param change.
- **Implementation**: Edit agent.rs (new enum variant), all skill .rs (return rich data), MI/debate/strategy (aggregate), meta_control (skill param mutations in rules or separate SkillParams).

### 2.5 Real WS Data Feeds (Binance Depth for Crypto)
- **Research Sources**: Binance WS docs (@depth streams for local book); Rust examples (tokio-tungstenite + binance-rs crates for depth/trade; "Easily connect to Binance WebSocket" tutorials; binance-futures-rs with typed DepthUpdate).
- **Key Insights**: Current loops.rs uses REST ticker/klines (polling, rate limits). Switch/add WS for true real-time + depth (enables realistic paper LOB + better MI signals like imbalance). Maintain local book per Binance "manage local order book" guide.
- **Validation**: Same real paper runs on BTC — compare decision latency/quality with live depth vs REST. Add book-derived features to skills (e.g., new "OrderBookImbalance" skill).
- **Build**: Add tokio-tungstenite + serde deps (or use existing binance crate); new feed module in orchestrator/loops or core; integrate into fast loop + PaperEngine.

Other research notes (from prior Research.md + searches): Multi-agent debate benefits from explicit skills (as we have); regret > pure reward for conservative trading; embedded DBs like Lance for edge/desktop (perfect for tredo redb + vector).

## 3. Concrete Implementation, Validation & Build Plans (Rust-First)

### Priority 1: Extended Self-Evo Validation (Observable Compounding)
**Implement**:
- In `crates/tredo-autonomous/src/meta_control.rs` or new `validation.rs`: Add `async fn run_extended_validation(&self, symbols: &[&str], cycles: usize, induce_regret: bool)`.
- For induce: Temporarily override SL tight in paper positions or use high-vol symbols.
- Track: Vec of (episode_id, regret, rules_snapshot). Persist to episode_store or new redb table.
- Expose in launcher `validate long --cycles 50 --induce`.
- Enhance COT with "EVOLUTION_METRIC: regret_trend=...".

**Validate (Real Paper Crypto)**:
- `source config/tredo.env; WATCHLIST=BTC,ETH ./tredo validate --long --induce-regret --cycles=100`
- Monitor: tail logs for RULE_ADAPT; query sqlite/redb for regret decreasing; compare two runs (with/without meta).
- Metrics: Avg regret/100 episodes, rule drift, paper P&L stability.

**Build**:
- `cargo test -p tredo-autonomous --test self_evo_validation` (add integration test).
- Run with live Ollama/Kronos + Binance.

### Priority 2: Realistic Paper (LOB) + Broker
**Implement**:
- In `crates/tredo-core/src/paper_engine.rs`: Add `struct LocalOrderBook { bids: BTreeMap, asks... }`; `fn apply_depth(&mut self, updates: DepthUpdate)`.
- On `place_order`: Walk book for realistic fill price/partial fills + slippage from depth.
- New `BinancePaperBroker` or extend PaperBroker (use WS depth).
- For live: `BinanceBroker` impl (use binance crate for signed orders, gated by !PAPER_MODE + feature).
- Update fast loop to feed depth.

**Validate**:
- Real paper on BTC: Trigger orders during live depth changes; assert fills more realistic (vs fixed slippage). Monitor P&L variance.
- `./tredo validate --realistic-paper`.

**Build**:
- Add deps: `tokio-tungstenite`, `binance-rs` (or futures variant) optional.
- Feature `realistic-paper` or `binance-ws`.
- Test against live Binance depth (no keys needed for public).

### Priority 3: Full LanceDB
**Implement**:
- In `crates/tredo-core/Cargo.toml`: Keep `lancedb = { version = "0.4", optional = true }`; pin `arrow = { version = "51", features = ["..."] }` to resolve chrono.
- Update `vector_memory.rs`: `#[cfg(feature = "lancedb")] use lancedb...`; connect, create_table with Arrow schema (vector: FixedSizeList<f32, 384 or whatever dim>, metadata fields).
- Migrate `search_by_vector` / `add` to Lance queries + filters (e.g., `symbol == 'BTC' AND regret > 0.5`).
- Fallback to JSON if !feature.

**Validate**:
- Real paper runs with/without feature: Compare trained recall quality in debate/MI (e.g., more relevant past episodes surface).
- `cargo build --features lancedb`; test recall speed.

**Build**:
- Resolve conflicts by testing arrow pins (see Medium/Rust examples).
- Update CI/launcher test to cover feature.
- Docs in vector_memory.rs.

### Priority 4: Richer Skills + Meta on Skills
**Implement**:
- `crates/tredo-core/src/agent.rs`: Add `AgentOutput::SkillResult { name: String, score: f64, note: String, confidence: f64 }`.
- Update all skills (sentiment etc.): Return rich data in execute.
- In `market_intelligence.rs` + `debate.rs`: Collect `Vec<SkillResult>`, push to COT, inject into reasoning/LLM context.
- In `meta_control.rs`: In review, correlate skill scores from episodes with regret; mutate e.g. skill weights or thresholds (store in new SkillConfig or extend DisciplineRules).

**Validate**:
- Paper crypto runs: Observe per-decision skill scores in logs/COT; post-run analysis shows e.g. "onchain score high → good outcome more often after meta tweak".
- `./tredo validate --track-skills`.

**Build**:
- Update Agent trait/impls.
- Add unit tests for skill aggregation.

### Priority 5: Binance WS Feeds
**Implement**:
- New module or in `crates/tredo-orchestrator/src/loops.rs`: WS client using tokio-tungstenite or binance crate.
- Subscribe `btcusdt@depth@100ms` + trades.
- Maintain local book; emit richer `MarketUpdate { price, book_imbalance, ... }`.
- Integrate into MI (new imbalance skill) + PaperEngine (for LOB sim).
- Fallback to REST.

**Validate**:
- Real paper: Compare decision quality/latency with WS depth vs current polling.
- Monitor book updates in logs during volatile periods.

**Build**:
- Cargo dep `tokio-tungstenite` + serde.
- Example from Medium/tms-dev-blog + Binance docs.
- Test with public WS (no keys).

## 4. Overall Build & Validation Workflow for Remaining

1. **Per item**: Implement behind feature flag or module (e.g., `#[cfg(feature = "lancedb")]`).
2. **Test locally**: `cargo clippy -D warnings; cargo test -p tredo-autonomous -p tredo-core`.
3. **Real paper crypto validation** (mandatory per user): 
   - Always `source config/tredo.env` (WATCHLIST=BTC,ETH; PAPER_MODE=true; Ollama + Kronos running).
   - `./tredo validate --extended` (or direct orchestrator + triggers).
   - Monitor: COT for new events, redb/sqlite for new episodes/regret trends, P&L.
   - Induce stress for self-evo.
4. **End-to-end**: Run full pipeline on crypto; assert no regression in debate/paper/reflect/meta.
5. **Update docs**: Extend Build.md §10 validation notes + roadmap table "Current State".
6. **Launcher**: Enhance `validate` subcommand for new modes (--lancedb, --lob-paper, --ws-feeds, --track-skills).
7. **Research persistence**: This file + future runs.

## 5. Recommended Immediate Order (per Build.md + post-validation reality)
1. Extended self-evo harness + long paper runs (quick win for credibility).
2. LanceDB (unlocks better memory for all above).
3. LOB paper + WS feeds (synergistic for realistic crypto paper).
4. Richer skills + meta-on-skills.
5. Broker adapters (after paper is "hands-off perfect").
6. Cleanup + production.

**Next command example**:
```bash
source config/tredo.env
./tredo validate --extended --cycles 50 --induce-regret  # Start here
```

All plans keep the system usable for real-time paper crypto testing throughout. Update this doc as items are completed.

**References** (from 2026 research):
- LanceDB Rust: docs.rs/lancedb, rig-lancedb examples, Medium RAG pipeline.
- Binance WS/LOB: developers.binance.com (depth streams + local book guide), binance-rs / tokio-tungstenite tutorials.
- Self-evo agents: arXiv Trace2Policy, RAGEN, TradingAgents/TradingGPT, meta-RL papers.
- LOB paper sim: GitHub tesser, genesis2025 repos.

This blueprint enables systematic completion while maintaining the validated real-time paper crypto foundation.