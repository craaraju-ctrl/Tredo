# TREDO — System Observation (trading start, workflow, data flow, memory)

> Grounded in a static read of the workspace (no compile/run was possible in the
> review sandbox — no Rust toolchain). File/function references are real.

## 1. How trading starts

Entry point is the **`tredo-orchestrator`** binary (`crates/tredo-orchestrator/src/main.rs`),
an `axum` HTTP + WebSocket server.

1. `initialize_autonomous_system()` (`tredo-autonomous/src/state.rs`) builds the
   `AutonomousOrchestrator`: shared state, portfolio, execution coordinator,
   memory handles, watchlist, event bus.
2. A `LoopManager` (in `main.rs`) holds the orchestrator + a `reqwest::Client` +
   the `EventBus`. `LoopManager::start()`:
   - opens a `watch` shutdown channel,
   - spawns three background tasks — **fast / medium / slow loops**
     (`tredo-orchestrator/src/loops.rs`),
   - sets `portfolio.trading_enabled = true`.
3. `LoopManager::stop()` sends shutdown on the watch channel, awaits the handles,
   and flips `trading_enabled = false`. So "trading running" == the three loops
   being alive with `trading_enabled = true`.

Trading mode (paper vs live) comes from `tredo_core::paper_engine::TradingMode`.

## 2. The workflow (the three temporal loops)

**Fast loop — 5s cadence** (`loops::fast_loop`): tactical.
- Reads the watchlist, fetches live prices in parallel (max 5 concurrent via a
  `Semaphore`) from Binance (crypto) / Yahoo (equities); on API error it falls
  back to a tiny "drift" of the last known price.
- Updates open-position P&L, publishes `MarketPrice` events on the bus and a
  `price` message on the TUI WebSocket, appends to 1-minute OHLCV history.
- Runs SL/TP monitoring and auto-exit via `orchestrator.execution.run(None)`.
- Periodically broadcasts a portfolio snapshot.

**Medium loop — 30s cadence** (`loops::medium_loop`, comment says "accelerated for
observation"): strategic. Per cycle:
- `execute_due_tasks()` runs scheduled agent tasks (market_scan, goal_review…).
- **Step 1:** compute `MarketMetricsMeter` per symbol (RSI, MACD, ATR%,
  confluence, regime hint) and update `state.market_regime`.
- **Step 1b:** refresh real OHLCV klines (skips if existing bars < 90s old).
- **Step 2:** run the agentic pipeline **sequentially, one symbol at a time**
  (parallel runs were removed — they caused LLM contention + portfolio races) via
  `pipeline_runner::run_single_quiet()`. On execution it captures a trade episode
  and emits `Signal` events.

**Slow loop** (`loops::slow_loop`): housekeeping / learning — reflection and
episode persistence (`state.memory.store_episode(...)`).

## 3. The decision pipeline (per symbol)

`run_full_pipeline_inner_quiet()` (`tredo-autonomous/src/orchestrator_pipeline.rs`)
+ phase bodies in `orchestrator_phases.rs`:

- **Preflight:** `ensure_market_data()` guarantees a live price + OHLCV bars.
- **OHLCV snapshot:** one `OhlcvSnapshot::capture()` so all verification layers
  (HardRulesGate, LLM, Kronos) see identical, same-timed data.
- **Phase 0** — skip if a position is already open on the symbol.
- **Phase 1** — discipline checks (`disciplined_core` / hard rules).
- **Phase 2** — market analysis.
- **Phase 3** — risk assessment.
- **Phase 4** — reflection (pulls past episodes — see memory).
- **Phase 5** — strategy decision.
- **Phase 6** — portfolio sizing + execution.

Every step writes a **Chain-of-Thought (COT)** step into `cot_store`
(`start_cot_chain` / `add_cot_step_quiet`). `quiet=true` in automated runs skips
per-agent COT to cut ~17 write-lock acquisitions per run. The whole pipeline is
wrapped in a 60s timeout.

## 4. Data flow & how it's connected

- **Market data in:** `tredo-market-data` + broker crates → `reqwest` HTTP →
  `ohlcv_history` / live price in `SharedState`.
- **Inter-component:** `tredo-eventbus` (`Arc<dyn EventBus>`) pub/sub —
  `MarketPrice`, `Signal`, `PortfolioSnapshot`, etc. Subjects in
  `event_subjects`. The orchestrator also pushes JSON over a WebSocket
  (`state.update_tx`) to the **`tredo-tui`** dashboard.
- **Metrics:** pipeline + outcomes are forwarded to `tredo-metrics`
  (`send_pipeline_event_to_metrics`, `send_trade_outcome_to_metrics`,
  `send_latency_to_metrics`).
- **Brokers:** `tredo-broker-*` (binance, alpaca, zerodha, upstox, angelone,
  5paisa) behind a common `broker` trait; paper trading via
  `tredo_core::paper_engine`.
- **Forecasts:** `KronosClient` (`tredo-core/src/kronos_client.rs`) → optional
  Python FastAPI **`kronos_service`** for short-horizon forecasts. Optional with
  graceful degradation.
- **Surfaces:** `tredo-server` (HTTP API), `tredo-tui` (terminal UI),
  `src-tauri` (desktop app), `tredo-watchdog`, `tredo-compliance`.

## 5. Memory — saving & retrieval

Three tiers:

**Hot / operational (in-process + redb).** `SharedState` holds `RwLock`-guarded
portfolio, `ohlcv_history`, watchlist, `market_regime`, `cot_store`. redb is the
embedded hot store for portfolio/rules/open episodes.

**Cold / history (SQLite).** `EpisodeStore` (`tredo-autonomous/src/episode_store.rs`)
→ `tredo_history.db` + `tredo_orders.db`. Append-only `ClosedEpisode` records
(entry/exit, pnl, outcome, `regret_score`, lesson, confluence, regime, session,
agent_reasoning…), COT logs, regret events, rule changes. "Zero RAM when idle —
SQLite pages load only on query."

**External memory API (port 3111).** `MemoryStore` (`memory.rs`), `VectorMemory`
(`vector_memory.rs`) and `AgentMemory` (`agentmemory.rs`) all talk to
`MEMORY_API_URL` (default `http://localhost:3111`). Each does a 10ms health check
at construction and sets `is_online`; **if offline, all calls become no-ops** —
the system degrades gracefully rather than failing.

- **Save:** slow loop + pipeline call `memory.store_episode(id, json)`; closed
  trades go to `EpisodeStore`; `VectorMemory::store()` writes an embedding +
  summary (`VectorEntry`) for each episode.
- **Retrieve:** `PatternRetriever::find_patterns()`
  (`tredo-autonomous/src/pattern_retriever.rs`) → `VectorMemory::search()` /
  `search_by_vector()` returns `SimilarResult`s (cosine similarity, regret score)
  for similar past setups; these feed **Phase 4 reflection** and strategy.

So the learning loop is: trade → `ClosedEpisode` (+ embedding) saved → future
pipelines recall similar episodes by vector similarity → reflection adjusts the
decision (and `weight_tuner` adjusts skill weights).

## 6. Notes for the fix pass

- No Rust toolchain was available in review, so nothing here was compiled. Fixes
  will be driven by your local `cargo build` / `cargo clippy` output.
- `target/` is 4.8 GB and gitignored; the review sandbox couldn't delete it
  (mount is read-only for build artifacts). Run `cargo clean` locally to reclaim
  it.
- 60 `#[allow(dead_code)]` sites and only 3 `TODO/unimplemented` markers — the
  code is feature-complete-ish; dead code is suppressed rather than removed.
  Real dead-code removal should follow `cargo clippy`/`-W dead_code` so we only
  delete what the compiler confirms is unused.
