# Contributing to tredo

Thank you for your interest in contributing to tredo — Trading Real-time Edge Decision Optimisation.

This document provides guidelines for setting up a development environment, coding conventions, and the PR process.

---

## Table of Contents

1. [Development Setup](#development-setup)
2. [Project Architecture](#project-architecture)
3. [Coding Conventions](#coding-conventions)
4. [Quality Gates](#quality-gates)
5. [Pull Request Process](#pull-request-process)
6. [Testing](#testing)

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

# Build the workspace
cargo build --workspace
```

### Environment

Copy `config/tredo.env.example` to `config/tredo.env` and edit as needed. Source it before running:

```bash
source config/tredo.env
```

---

## Project Architecture

tredo is a **Rust-first** workspace organised into these crates:

| Crate | Purpose |
|-------|---------|
| `tredo-core` | Foundation: DisciplinedCore rules, memory (redb), LLM client, AgentSkill trait, paper engine |
| `tredo-autonomous` | Agent hierarchy, debate, skills implementations, reflection, meta-control, state |
| `tredo-orchestrator` | Temporal loops (fast/medium/slow), HTTP/WS API server |
| `tredo-tui` | Primary Terminal UI (ratatui) — COT tree, portfolio, agent views |
| `tredo-server` | Optional production HTTP server |
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
- **LLM** = used sparingly, only after debate + rules + memory gates

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

### Memory

- Use `redb` for hot operational state
- Use SQLite (`episode_store`) for persistent episode journal
- Use vector memory (`VectorMemory`) for semantic recall of past episodes
- Use `agentmemory` client for cross-session long-term intelligence

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

---

## Pull Request Process

1. **Branch** — Create a feature branch from `main`:
   ```bash
   git checkout -b feat/my-feature
   ```

2. **Commit** — Use descriptive commit messages:
   - `feat: ...` for new features
   - `fix: ...` for bug fixes
   - `refactor: ...` for code improvements
   - `docs: ...` for documentation
   - `chore: ...` for maintenance

3. **Run quality gates** — Ensure all checks pass locally (see [Quality Gates](#quality-gates))

4. **Open a PR** — Against `main` with a clear description of the changes

5. **CI must pass** — The GitHub Actions pipeline must be green before merge

---

## Testing

### Unit Tests

```bash
# Core tests
cargo test -p tredo-core

# Agent tests
cargo test -p tredo-autonomous
```

### Real-Time Paper Validation

The project validates against live Binance data (not simulation):

```bash
source config/tredo.env
./tredo validate --extended --induce-regret
```

This runs the full autonomous system and measures self-evolution metrics.

---

## Need Help?

- Open a [GitHub Discussion](https://github.com/craaraju-ctrl/Tredo/discussions)
- Refer to [Build.md](Build.md) for the complete build and evolution guide
- Check [Research.md](Research.md) for the project's research foundation

---

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
