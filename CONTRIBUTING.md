# Contributing to tredo

Thank you for your interest in contributing to tredo — Trading Real-time Edge Decision Optimisation.

This document provides guidelines for setting up a development environment, coding conventions, and the PR process.

---

## Table of Contents

1. [Development Setup](#development-setup)
2. [Branching Strategy](#branching-strategy)
3. [Project Architecture](#project-architecture)
4. [Coding Conventions](#coding-conventions)
5. [Quality Gates](#quality-gates)
6. [Pull Request Process](#pull-request-process)
7. [Testing](#testing)

---

## Development Setup

### Prerequisites

- **Rust** 1.75+ (stable) — install via [rustup](https://rustup.rs/)
  ```bash
  rustup toolchain install stable --component rustfmt --component clippy
  ```
- **Python** 3.10+ (for the Kronos forecast sidecar)
- **Ollama** (for LLM inference during development)
- A good terminal that supports ratatui (any modern terminal emulator)

### Clone & Build

```bash
git clone https://github.com/craaraju-ctrl/Tredo.git
cd Tredo

# Copy environment template
cp config/tredo.env.example config/tredo.env

# Build the workspace (including new runtime and broker crates)
cargo build --workspace
```

### Environment

Copy `config/tredo.env.example` to `config/tredo.env` and edit as needed. Source it before running:

```bash
source config/tredo.env
```

---

## Branching Strategy

We use a **Git Flow-inspired** branching model adapted for a fast-moving Rust project.

```
main          ──── stable, production-ready, tagged releases
    ↑
develop       ──── integration branch; all features merge here first
    ↑
feat/*        ──── individual feature branches (from develop)
fix/*         ──── bug fix branches (from develop or main)
docs/*        ──── documentation-only changes
refactor/*    ──── structural refactors with no behaviour change
release/*     ──── release preparation (from develop or main)
hotfix/*      ──── emergency fixes (from main)
```

### Branch Rules

| Branch | Purpose | Merge Target | CI Required |
|--------|---------|-------------|-------------|
| `main` | Stable production code | — | ✅ Full gate |
| `develop` | Integration / next release | `main` | ✅ Full gate |
| `feat/*` | New features | `develop` | ✅ Full gate |
| `fix/*` | Bug fixes | `develop` (or `main` for hotfixes) | ✅ Full gate |
| `docs/*` | Documentation only | `develop` | ✅ Format + build |
| `refactor/*` | Code restructuring | `develop` | ✅ Full gate |
| `release/*` | Release prep / version bump | `main` | ✅ Full gate + release build |
| `hotfix/*` | Critical production fixes | `main` + `develop` | ✅ Full gate |

### Creating a Feature Branch

```bash
# Always branch from develop (unless it's a hotfix)
git checkout develop
git pull origin develop
git checkout -b feat/my-new-feature

# Work, commit, push
git push -u origin feat/my-new-feature
# Then open a PR against develop
```

### Release Branches

When `develop` is ready for release:

```bash
git checkout develop
git pull origin develop
git checkout -b release/v0.3.0
# Version bump, final changelog, release notes
git push -u origin release/v0.3.0
# Open PR → main
```

### Hotfix Branches

For critical bugs in production:

```bash
git checkout main
git pull origin main
git checkout -b hotfix/critical-fix
# Fix, commit, PR → main
# After merge: also cherry-pick or merge into develop
```

---

## Project Architecture

tredo is a **Rust-first** workspace organised into these crates:

| Crate | Purpose |
|-------|---------|
| `tredo-core` | Foundation: DisciplinedCore rules, memory (redb), LLM client, AgentSkill trait, paper engine, BrokerAdapter trait, backtest engine |
| `tredo-autonomous` | Agent hierarchy, debate, skills implementations, reflection, meta-control, state, pipeline, new: DebateOrchestrator, OutcomeProcessor, RegimeClassifier |
| `tredo-orchestrator` | Temporal loops (fast/medium/slow), HTTP/WS API server |
| `tredo-tui` | Primary Terminal UI (ratatui) — COT tree, portfolio, agent views, policy cache, backtest, health, performance |
| `tredo-server` | Production HTTP server (Axum + broker registry + paper/live mode switching) |
| `tredo-runtime` | **Unified runtime engine** — event-driven, multi-mode (paper/live/backtest/validate/research), world model, policy cache, active learner, broker plugin system, introspector |
| `tredo-broker-alpaca` | Alpaca Markets API v2 broker adapter (US equities + crypto, paper + live) |
| `tredo-broker-zerodha` | Zerodha Kite Connect v3 broker adapter (India equities + derivatives) |
| `src-tauri` | Secondary desktop UI (Tauri + vanilla JS) |

**External services** (run separately):
- **Ollama** — local LLM inference
- **Kronos** — Python time-series forecast sidecar (`kronos_service/`)

### Core Philosophy

```
Rules + Memory > Pure Prompting
```

- **Rules** (DisciplinedCore) = hard non-negotiable trading gates in Rust
- **Skills** (AgentSkill trait) = pluggable deterministic capabilities
- **Trained Memory** = vector RAG + episode store for past outcomes
- **Policy Cache** = learned (features → action → outcome) lookup table that short-circuits debate
- **LLM** = used sparingly, only after debate + rules + memory + cache gates

---

## Coding Conventions

### Rust-First

Core components must be implemented in Rust. The only justified exception is the
Kronos forecast service (Python with Chronos-Bolt), which has a graceful Rust
fallback client. Any new non-Rust dependency requires explicit justification.

### Code Style

- Run `cargo fmt --all` before committing
- Follow standard Rust naming conventions:
  - `snake_case` for functions, variables, modules
  - `CamelCase` for types, traits, enums
  - `SCREAMING_CASE` for constants
- Prefer `Result<T, Box<dyn Error + Send + Sync>>` for fallible functions
- Use `async`/`await` with `tokio` for asynchronous code
- Mark deprecated or transitional code clearly with `#[deprecated]` and comments

### Agent Structure

New agent capabilities should follow the existing patterns:

- **Deterministic sub-agents** — pure logic, no LLM calls, implement as `AgentSkill`
- **Main agents** — orchestrate sub-agents, may use LLM for synthesis
- **Skills** — implement the `AgentSkill` trait from `tredo-core/src/skills.rs`
- **Debate roles** — extend the debate system in `tredo-autonomous/src/debate.rs`
- **Runtime modules** — add to `tredo-runtime/src/` with proper event bus integration

### Memory

- Use `redb` for hot operational state
- Use SQLite (`episode_store`) for persistent episode journal
- Use vector memory (`VectorMemory`) for semantic recall of past episodes
- Use `agentmemory` client for cross-session long-term intelligence
- Use `PolicyCache` (in `tredo-runtime`) for learned trading memory

---

## Quality Gates

Every PR must pass these checks locally before submission:

```bash
# 1. Formatting
cargo fmt --all -- --check

# 2. Linting (deny warnings)
cargo clippy --workspace --all-targets -- -D warnings

# 3. Tests
cargo test --workspace

# 4. Build (debug is fast enough for CI parity)
cargo check --workspace
```

The CI pipeline runs these same gates on every push and PR. Results are visible
in the [Actions tab](https://github.com/craaraju-ctrl/Tredo/actions).

### CI Jobs

| Job | What it checks |
|-----|---------------|
| Rust checks | `fmt` → `clippy` → `test` → `build --release` |
| Tauri | Checks `tredo-tauri` crate compiles with GTK deps |
| Kronos | Validates Python service syntax |
| Broker adapters | Tests `tredo-broker-alpaca` and `tredo-broker-zerodha` in paper mode |

---

## Pull Request Process

1. **Branch** — Create a feature branch from `develop` (or `main` for hotfixes):
   ```bash
   git checkout develop
   git pull origin develop
   git checkout -b feat/my-feature
   ```

2. **Commit** — Use descriptive commit messages following [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat: ...` for new features
   - `fix: ...` for bug fixes
   - `refactor: ...` for code improvements
   - `docs: ...` for documentation
   - `chore: ...` for maintenance
   - `test: ...` for test-only changes
   - `ci: ...` for CI/CD changes
   - `perf: ...` for performance improvements

3. **Run quality gates** — Ensure all checks pass locally (see [Quality Gates](#quality-gates))

4. **Open a PR** — Against `develop` with a clear description of the changes:
   - Title: `feat: add unified runtime engine` or `fix: resolve memory leak in vector store`
   - Description: What changed, why, and how it was tested
   - Link any related issues

5. **Code review** — At least one approving review required before merge

6. **CI must pass** — The GitHub Actions pipeline must be green before merge

7. **Merge** — Use **squash and merge** for feature branches to keep `develop` history clean. Use **merge commit** for release branches to preserve branch topology.

---

## Testing

### Unit Tests

```bash
# Core tests
cargo test -p tredo-core

# Agent tests
cargo test -p tredo-autonomous

# Runtime tests
cargo test -p tredo-runtime

# Broker adapter tests (paper mode)
cargo test -p tredo-broker-alpaca -p tredo-broker-zerodha

# Integration tests (13 tests covering all agent groups)
cargo test -p tredo-autonomous --test tredo_integration
```

### All Tests

```bash
# Full workspace
cargo test --workspace
```

### Real-Time Paper Validation

The project validates against live Binance data (not simulation):

```bash
# Via the new unified CLI
cargo run -p tredo-runtime -- --mode validate --cycles 50

# Or via legacy launcher
source config/tredo.env
PORT=8082 cargo run -p tredo-orchestrator &
cargo run -p tredo-tui   # Open TUI to observe live
curl -X POST http://localhost:8082/api/trigger_cycle -H "Content-Type: application/json" -d '{"symbol":"BTC"}'
```

This runs the full autonomous system and produces 20+ COT entries across all sub-agents.

---

## Need Help?

- Open a [GitHub Discussion](https://github.com/craaraju-ctrl/Tredo/discussions)
- Refer to [Build.md](Build.md) for the complete build and evolution guide
- Check [Research.md](Research.md) for the project's research foundation

---

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
