# TREDO — Engineering Loop Completion Plan

Turns the 2026 blueprint into concrete, file-level work. Each item is sized to be
implemented, then verified by **your** `cargo build` / `cargo clippy` / tests
(the review environment has no Rust toolchain). Ordered by leverage.

## Status legend
- `EXISTS` — implemented and wired.
- `PARTIAL` — present but not fully wired / not observable.
- `GAP` — needs building.

---

## 0. DONE in this pass — make the engineering loop observable  ✅
**Problem:** the self-evolution loop *is* wired in `slow_loop`
(`crates/tredo-orchestrator/src/loops.rs`) — knowledge-graph rebuild → deep
reflection → `MetaControlAgent::weekly_review` → `EvolvedMetaControl::evaluate_and_adapt`
→ `check_and_revert_if_degraded` — but it was pinned to a hard-coded **24h sleep**,
so it never fires in observation/validation runs.

**Change made:** all three loop cadences are now env-configurable via
`loop_cadence_secs()` (`TREDO_FAST_LOOP_SECS` / `TREDO_MEDIUM_LOOP_SECS` /
`TREDO_SLOW_LOOP_SECS`), defaults unchanged. Documented in `config/tredo.env.example`.

**Verify:** `cargo build -p tredo-orchestrator`, then run with
`TREDO_SLOW_LOOP_SECS=120` and watch for `Meta-review completed` / `RULE_ADAPT` /
`RULE_REVERT` log lines within minutes.

---

## 1. Extended self-evolution validation (observable compounding) — PARTIAL
**Goal:** a repeatable harness that runs N cycles, optionally induces regret, and
reports whether avg regret trends down and rules adapt.

- `crates/tredo-autonomous/src/self_evolution.rs` already has
  `SelfEvolutionValidator::run_extended_validation` and `SelfEvolutionReport`.
  Confirm it's reachable from a CLI subcommand.
- Add a launcher subcommand `validate --long --cycles N --induce-regret` in
  `crates/tredo-autonomous/src/bin/tredo_cli.rs` (and/or the `tredo` script) that
  drives it against the paper engine.
- Persist `(episode_id, regret, rules_snapshot)` per cycle (reuse
  `episode_store.rs` rule-change tables) and emit a COT line
  `EVOLUTION_METRIC: regret_trend=...`.
- **Test:** `cargo test -p tredo-autonomous` — add an integration test that runs
  ~20 synthetic episodes and asserts the report aggregates regret buckets.

## 2. Realistic paper execution (local order book + slippage) — GAP
**Goal:** replace fixed-% slippage with depth-walked fills.

- `crates/tredo-core/src/paper_engine.rs`: add `LocalOrderBook { bids: BTreeMap,
  asks: BTreeMap }` + `apply_depth(&mut self, DepthUpdate)`; on `place_order`
  walk levels for fill price + partial fills.
- Feed depth from the fast loop (see item 5).
- Keep current behavior behind a flag (`TREDO_REALISTIC_PAPER`) so it's opt-in
  until validated.
- **Test:** unit test that a market order against a known book ladder produces
  the expected VWAP fill and remaining qty.

## 3. Real-time Binance WS depth feed — GAP
- New module `crates/tredo-orchestrator/src/feeds/binance_ws.rs` (or in
  `tredo-market-data`) using `tokio-tungstenite`: subscribe `@depth@100ms`,
  bootstrap with REST snapshot, drop stale events by `updateId` (per Binance
  "manage local order book").
- Push updates into `SharedState` (new `order_books: RwLock<HashMap<String,
  LocalOrderBook>>`) and into the paper engine.
- Derive an `OrderBookImbalance` skill for the pipeline.
- **Deps:** add `tokio-tungstenite`, feature-gate `binance-ws`.

## 4. Full LanceDB vector memory — PARTIAL (JSON fallback today)
- `crates/tredo-core/src/vector_memory.rs` currently brute-forces cosine over a
  JSON store via the external memory API. Add `#[cfg(feature = "lancedb")]` path:
  Arrow schema with `FixedSizeList<f32, DIM>` + metadata (symbol, regret, lesson,
  ts), vector search + filters; JSON path stays as fallback.
- Resolve the known arrow/chrono conflict already noted in `Cargo.toml`
  (`chrono` pinned `<0.4.40`); pin `arrow = 51` under the feature.
- **Verify:** `cargo build -p tredo-core --features lancedb`.

## 5. Richer skills + meta-adaptation of skills — PARTIAL
- `crates/tredo-core/src/agent.rs`: add a structured `SkillResult { score, note }`
  variant to `AgentOutput` so skills return data instead of side effects.
- Aggregate skill contributions in market-intelligence/debate/strategy; correlate
  with post-trade regret in `meta_control.rs` to mutate skill weights
  (`weight_tuner.rs` already applies weight snapshots).
- **Test:** assert a high-regret cluster lowers the offending skill's weight.

## 6. Cleanup (after the above compile green) — GAP
- Run `cargo clippy --all-targets -- -W dead_code`; remove the 60
  `#[allow(dead_code)]` sites that clippy confirms are truly unused. Do **not**
  delete blind — only what the compiler flags.
- `cargo clean` to drop the 4.8 GB `target/` (sandbox couldn't delete it).
- Confirm no leftover duplicate/`.bak` modules; ensure `Cargo.toml` members match
  on-disk crates.

---

## Execution protocol (because the review env can't compile)
1. I implement an item end-to-end in code.
2. You run `cargo build` / `cargo clippy` / `cargo test` and paste output (or drop
   `build.log` in this folder).
3. I fix against the real errors. Repeat until green, then move to the next item.

Recommended order: **1 → 2 → 3 → 4 → 5 → 6** (each builds on the last; 2 depends
on 3's depth feed for live data but can be unit-tested with synthetic books first).
