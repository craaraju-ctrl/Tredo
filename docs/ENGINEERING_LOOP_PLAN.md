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

## 1. Extended self-evolution validation (observable compounding) — MOSTLY DONE
**Goal:** a repeatable harness that runs N cycles, optionally induces regret, and
reports whether avg regret trends down and rules adapt.

DONE this pass:
- `self_evolution.rs` already had `run_extended_validation` + `SelfEvolutionReport`.
- Added CLI subcommand `tredo-cli self-evolve [cycles] [--induce] [--symbols ...]`
  wiring it to the real orchestrator.
- Made `compute_buckets` a pure fn and added 4 deterministic unit tests for the
  bucket / regret-trend / win-rate math (run in CI via `cargo test`).

REMAINING (the one real gap): `--induce` currently only *logs*
`INDUCED_REGRET_SL_PCT` — it does not yet tighten actual stops, so it can't force
high-regret sequences. To finish: gate a tight-SL override in the execution/risk
path on an env flag (e.g. `TREDO_INDUCE_REGRET_SL_PCT`) that the validator sets,
so induced runs produce real losses → real rule adaptations. Needs the
build-verify loop (touches risk sizing). Optional: emit an
`EVOLUTION_METRIC: regret_trend=...` COT line per cycle.

## 2. Realistic paper execution (local order book + slippage) — DONE ✅ (needs a feed)
**Re-audit (2026-06-27):** already implemented in `crates/tredo-core/src/paper_engine.rs`:
- `LocalOrderBook` (sorted bids/asks) with `apply_depth_update(bids, asks, update_id)`,
  `market_buy`/`market_sell` walking the book → `FillResult` (avg fill price,
  filled qty, slippage %, levels consumed, partial-fill flag).
- `PaperEngine` holds `order_books: RwLock<HashMap<String, LocalOrderBook>>` plus
  `apply_depth_snapshot`, `apply_depth_update`, `get_order_book`,
  `estimate_realistic_slippage`.
- `place_order` **already walks the book** for `OrderType::Market` when
  `config.realistic_paper_enabled == true` and the book has data; otherwise it
  falls back to fixed-% slippage.
- Unit-test surface exists.

**Remaining for item 2:** nothing structural — it just needs (a) a live depth
source populating `order_books` (item 3) and (b) `realistic_paper_enabled = true`.
Until a feed exists, `place_order` correctly falls back to fixed slippage.

## 3. Feed depth into the order book — GAP (this unlocks item 2 at runtime)
The plumbing exists (`apply_depth_snapshot` / `apply_depth_update`); nothing calls
it yet. Two options, smallest first:

**3a. REST snapshot (low risk, reuse existing code).** `main.rs` already fetches
`https://api.binance.com/api/v3/depth` for the `/depth` handler. In the fast loop,
for each crypto symbol, fetch the depth snapshot every N ticks and call
`paper_engine.apply_depth_snapshot(symbol, bids, asks)`. No new deps. Gate behind
`realistic_paper_enabled`. This makes realistic fills work today.

**3b. WS stream (lower latency, more code).** New module using `tokio-tungstenite`
(already in the tree): subscribe `@depth@100ms`, bootstrap with REST snapshot,
drop stale events by `updateId` per Binance "manage local order book". Push diffs
via `apply_depth_update`. Optionally derive an `OrderBookImbalance` skill.

**Recommendation:** do 3a first (small, reuses proven REST code, immediately
enables item 2), then 3b if latency matters. Both must go through the build-verify
loop — they touch the hot path and P&L.

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
