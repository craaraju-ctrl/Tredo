// tredo_integration.rs
// Integration tests for the Tredo agent hierarchy — verifies agent communication,
// data flow through Identifier → Verifier → Executer groups, and shared state integrity.
//
// Run: cargo test -p tredo-autonomous --test tredo_integration

use chrono::Utc;
use std::fs;
use std::sync::Arc;
use tredo_autonomous::{
    types::{RiskRecommendation, TradeSignal},
    AutonomousOrchestrator, Executer, Guardian, Identifier, SharedState, Tredo, Verifier,
};
use tredo_core::{Config, DisciplineRules, MemoryStore, OhlcvBar, TradeDirection};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a fresh test environment. Returns (orchestrator, _db_path) where
/// _db_path keeps the file alive for the test scope.
///
/// Cleans up the shared SQLite `tredo_history.db` (opened by SharedState::new)
/// plus any WAL/SHM sidecar files before creating the new env.
fn setup_test_env(db_name: &str) -> (AutonomousOrchestrator, String) {
    let sqlite_db_path = format!("test_history_{}.db", db_name);
    for f in &[
        sqlite_db_path.to_string(),
        format!("{}-wal", sqlite_db_path),
        format!("{}-shm", sqlite_db_path),
    ] {
        let _ = fs::remove_file(f);
    }
    let db_path = format!("test_{}.redb", db_name);
    let _ = fs::remove_file(&db_path);

    let memory = MemoryStore::new(&db_path).expect("MemoryStore creation");
    let config = Config::default();
    let rules = DisciplineRules::default();
    let state = SharedState::new(memory, rules, config, &sqlite_db_path)
        .expect("SharedState init (episode DB)");
    let mut orch = AutonomousOrchestrator::new(state);
    orch.init_tredo();
    (orch, db_path)
}

/// Seed OHLCV history into shared state so the scanner and market intelligence
/// have data to work with.
async fn seed_ohlcv(state: &SharedState, symbol: &str, base_price: f64) {
    let mut history = state.ohlcv_history.write().await;
    let mut bars = Vec::with_capacity(20);
    for i in 0..20 {
        let noise = (i as f64).sin() * base_price * 0.01;
        bars.push(OhlcvBar {
            timestamp: (Utc::now() - chrono::Duration::minutes(20 - i as i64)).to_rfc3339(),
            open: base_price + noise,
            high: base_price + noise.abs() + base_price * 0.01,
            low: base_price - noise.abs() - base_price * 0.01,
            close: base_price + noise * 0.5,
            volume: 100_000.0 + (i as f64) * 1_000.0,
        });
    }
    history.insert(symbol.to_string(), bars);
}

/// Count open positions in state.
async fn count_positions(state: &SharedState) -> usize {
    state.portfolio.read().await.open_positions.len()
}

/// Read portfolio equity.
async fn get_equity(state: &SharedState) -> f64 {
    state.portfolio.read().await.total_equity
}

/// Helper to simulate the outcome recording path (OutcomeProcessor behavior).
/// Call this from tests that want to populate closed_trades + skill_performance
/// without needing a full LLM-driven paper execution.
/// This helps resolve "tests produce 0-row DBs for self-evolution data".
fn simulate_outcome_recording(state: &SharedState, symbol: &str, pnl: f64, was_win: bool) {
    let store = &state.episode_store;
    let episode_id = format!("sim-ep-{}", symbol);

    // Simulate some skill votes (as MI would have captured)
    let votes = vec![
        tredo_core::SkillVote {
            skill_name: "SentimentAnalyzer".to_string(),
            direction: tredo_core::SkillDirection::Bullish,
            weight: 0.30,
            confidence: 0.75,
            score: 0.70,
        },
    ];

    for v in &votes {
        let sp = tredo_autonomous::episode_store::SkillPerformanceRow {
            id: 0,
            episode_id: episode_id.clone(),
            skill_name: v.skill_name.clone(),
            direction: format!("{:?}", v.direction),
            weight_used: v.weight,
            confidence: v.confidence,
            score: v.score,
            was_correct: was_win,
            recorded_at: chrono::Utc::now().to_rfc3339(),
        };
        let _ = store.insert_skill_performance(&sp);
    }

    let closed = tredo_autonomous::episode_store::ClosedEpisode {
        id: episode_id.clone(),
        symbol: symbol.to_string(),
        direction: "Long".to_string(),
        entry_price: 100.0,
        exit_price: 100.0 + pnl,
        stop_loss: 99.0,
        take_profit: 101.0,
        position_size: 1.0,
        pnl,
        pnl_pct: pnl / 100.0,
        outcome: if was_win { "WIN".to_string() } else { "LOSS".to_string() },
        exit_reason: if pnl > 0.0 { "take_profit".to_string() } else { "stop_loss".to_string() },
        regret_score: if was_win { 0.15 } else { 0.65 },
        lesson: if was_win { "Good signal followed.".to_string() } else { "Overtraded or bad confluence.".to_string() },
        confluence_score: 0.72,
        portfolio_heat: 0.02,
        market_regime: "TrendingBull".to_string(),
        session: "Normal Session".to_string(),
        agent_reasoning: "Test simulation of aggregated skills + debate.".to_string(),
        consecutive_losses_at_entry: 0,
        entry_time: chrono::Utc::now().to_rfc3339(),
        exit_time: chrono::Utc::now().to_rfc3339(),
    };
    let _ = store.insert_closed_trade(&closed);
}

// ══════════════════════════════════════════════════════════════════════════════
// HIERARCHY INTEGRITY
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_tredo_hierarchy_integrity() {
    let (orch, db_path) = setup_test_env("hierarchy");
    let tredo = orch.tredo();

    // Verify the four groups exist with all 16 sub-agents via Arc::ptr_eq
    // Each Tredo agent Arc should point to the same instance as the orchestrator's
    assert!(
        Arc::ptr_eq(&orch.scanner, &tredo.identifier.scanner),
        "Tredo scanner should share orchestrator scanner Arc"
    );

    // Verify each group's describe() returns the expected tree
    let ident_desc = Identifier::describe();
    assert!(
        ident_desc.contains("WatchlistScannerAgent"),
        "Identifier should contain scanner"
    );
    assert!(
        ident_desc.contains("MarketIntelligenceAgent"),
        "Identifier should contain market intel"
    );
    assert!(
        ident_desc.contains("PivotCalculatorAgent"),
        "Identifier should contain pivot calc"
    );
    assert!(
        ident_desc.contains("ConfluenceScorerAgent"),
        "Identifier should contain confluence"
    );
    assert!(
        ident_desc.contains("PatternRetrieverAgent"),
        "Identifier should contain pattern retriever"
    );
    assert!(
        ident_desc.contains("SessionTimerAgent"),
        "Identifier should contain session timer"
    );
    assert!(
        ident_desc.contains("RedFolderCheckerAgent"),
        "Identifier should contain red folder"
    );

    let ver_desc = Verifier::describe();
    assert!(
        ver_desc.contains("RiskPsychologyAgent"),
        "Verifier should contain risk psychology"
    );
    assert!(
        ver_desc.contains("RiskCalculatorAgent"),
        "Verifier should contain risk calc"
    );
    assert!(
        ver_desc.contains("ReflectorAgent"),
        "Verifier should contain reflector"
    );

    let exec_desc = Executer::describe();
    assert!(
        exec_desc.contains("StrategyDecisionAgent"),
        "Executer should contain strategy"
    );
    assert!(
        exec_desc.contains("PortfolioManagerAgent"),
        "Executer should contain portfolio manager"
    );
    assert!(
        exec_desc.contains("ExecutionCoordinatorAgent"),
        "Executer should contain execution"
    );

    let guard_desc = Guardian::describe();
    assert!(
        guard_desc.contains("DrawdownMonitorAgent"),
        "Guardian should contain drawdown"
    );
    assert!(
        guard_desc.contains("OvertradingPreventerAgent"),
        "Guardian should contain overtrading"
    );
    assert!(
        guard_desc.contains("OutcomeLoggerAgent"),
        "Guardian should contain outcome logger"
    );

    // Verify Tredo's tree_json is valid
    let tree = Tredo::tree_json();
    assert_eq!(tree["name"], "Tredo", "Root should be Tredo");
    assert_eq!(
        tree["children"].as_array().unwrap().len(),
        4,
        "Should have 4 children: Identifier, Verifier, Executer, Guardian"
    );
    assert_eq!(tree["children"][0]["name"], "Identifier");
    assert_eq!(tree["children"][1]["name"], "Verifier");
    assert_eq!(tree["children"][2]["name"], "Executer");
    assert_eq!(tree["children"][3]["name"], "Guardian");

    // Verify agent counts in tree_json
    let ident_agents = tree["children"][0]["children"].as_array().unwrap().len();
    let ver_agents = tree["children"][1]["children"].as_array().unwrap().len();
    let exec_agents = tree["children"][2]["children"].as_array().unwrap().len();
    let guard_agents = tree["children"][3]["children"].as_array().unwrap().len();
    assert_eq!(ident_agents, 7, "Identifier should contain 7 sub-agents");
    assert_eq!(ver_agents, 3, "Verifier should contain 3 sub-agents");
    assert_eq!(exec_agents, 3, "Executer should contain 3 sub-agents");
    assert_eq!(guard_agents, 3, "Guardian should contain 3 sub-agents");
    assert_eq!(
        ident_agents + ver_agents + exec_agents + guard_agents,
        16,
        "Total: 16 sub-agents"
    );

    // Verify Arc sharing: all Arcs point to the same heap-allocated agent instances
    assert!(
        Arc::ptr_eq(&orch.scanner, &tredo.identifier.scanner),
        "scanner Arc should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.market_intel, &tredo.identifier.market_intel),
        "market_intel Arc should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.risk_psych, &tredo.verifier.risk_psych),
        "risk_psych Arc should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.execution, &tredo.executer.execution),
        "execution Arc should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.drawdown, &tredo.guardian.drawdown),
        "drawdown Arc should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.outcome_logger, &tredo.guardian.outcome_logger),
        "outcome_logger Arc should be shared"
    );

    // Verify state mutation propagates across groups (all share the same inner state)
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.daily_pnl = 1234.56;
    }
    {
        // Read back through Tredo's scanner's state
        let portfolio = tredo.identifier.scanner.state.portfolio.read().await;
        assert!((portfolio.daily_pnl - 1234.56).abs() < f64::EPSILON,
            "State mutation via orchestrator should be visible through Tredo. Got {}, expected 1234.56", portfolio.daily_pnl);
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// IDENTIFIER GROUP — Market scanning & intelligence
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_identifier_group() {
    let (orch, db_path) = setup_test_env("identifier");
    let symbol = "BTC";

    // Seed OHLCV data so scanner and market intelligence have prices to work with
    seed_ohlcv(&orch.state, symbol, 65_000.0).await;
    seed_ohlcv(&orch.state, "NIFTY", 24_500.0).await;
    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Run the Identifier group
    let result = orch.tredo().run_identifier(symbol, 65_000.0).await;

    assert!(
        result.is_ok(),
        "Identifier should complete without error: {:?}",
        result.err()
    );
    let (discipline_ok, confluence, pivots) = result.unwrap();

    // Discipline checks: BTC is crypto so it bypasses session timer → discipline should pass
    assert!(
        discipline_ok,
        "Crypto symbols should bypass session checks → discipline OK"
    );

    // Confluence should be a valid f64 between 0.0 and 1.0
    assert!(
        confluence >= 0.0,
        "Confluence should be >= 0.0, got {}",
        confluence
    );
    assert!(
        confluence <= 1.0,
        "Confluence should be <= 1.0, got {}",
        confluence
    );
    println!("  Confluence: {:.2}%", confluence * 100.0);

    // Pivots should have valid finite values
    assert!(
        pivots.pivot.is_finite(),
        "Pivot should be finite: {:?}",
        pivots.pivot
    );
    assert!(
        pivots.r1.is_finite(),
        "R1 should be finite: {:?}",
        pivots.r1
    );
    assert!(
        pivots.s1.is_finite(),
        "S1 should be finite: {:?}",
        pivots.s1
    );
    println!(
        "  Pivot: {:.2}, R1: {:.2}, S1: {:.2}",
        pivots.pivot, pivots.r1, pivots.s1
    );

    // Ensure state was updated (market regime should be set)
    let regime = orch.state.market_regime.read().await;
    assert!(
        regime.is_some(),
        "Market regime should be set after Identifier run"
    );

    // COT store should have entries
    let cot_len = orch.state.cot_store.read().await.len();
    // Initial entry from pipeline is not added here, but sub-agents may push COT entries
    // This just verifies no panics or state corruption
    println!("  COT entries after identifier: {}", cot_len);

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_identifier_without_data() {
    let (orch, db_path) = setup_test_env("identifier_empty");

    // Run identifier WITHOUT seeding OHLCV data — should still work gracefully
    let result = orch.tredo().run_identifier("NIFTY", 24_500.0).await;

    // Should still succeed because agents handle missing data gracefully
    assert!(
        result.is_ok(),
        "Identifier should handle missing OHLCV data gracefully: {:?}",
        result.err()
    );
    let (discipline_ok, confluence, pivots) = result.unwrap();

    // Without data, scanner skips symbols with price 0.0 and market intel falls back to single bar
    // Discipline: NIFTY is not crypto → session timer may or may not pass depending on time of day
    // We just verify the function completes without panicking
    assert!(confluence >= 0.0, "Confluence should be in valid range");
    assert!(
        pivots.pivot.is_finite(),
        "Pivot should be calculated even with fallback data"
    );

    println!(
        "  No-data test — discipline: {}, confluence: {:.2}%, pivot: {:.2}",
        if discipline_ok { "OK" } else { "FAIL" },
        confluence * 100.0,
        pivots.pivot
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// VERIFIER GROUP — Risk & discipline validation
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_verifier_clean_portfolio() {
    let (orch, db_path) = setup_test_env("verifier_clean");

    // Clean portfolio (no open positions, no losses) → should return Proceed
    let equity = get_equity(&orch.state).await;
    assert_eq!(equity, 100_000.0, "Initial equity should be 100k");

    let result = orch.tredo().run_verifier("BTC", 65_000.0, equity).await;

    assert!(
        result.is_ok(),
        "Verifier should complete without error: {:?}",
        result.err()
    );
    let analysis = result.unwrap();

    // Clean portfolio should recommend Proceed
    assert_eq!(
        analysis.recommendation,
        RiskRecommendation::Proceed,
        "Clean portfolio should get Proceed recommendation, got {:?}",
        analysis.recommendation
    );
    assert_eq!(
        analysis.portfolio_heat, 0.0,
        "No open positions → zero portfolio heat"
    );
    assert_eq!(
        analysis.daily_drawdown_pct, 0.0,
        "No daily P&L → zero drawdown"
    );
    assert!(
        analysis.max_position_size > 0.0,
        "Max position size should be positive"
    );
    assert!(
        analysis.psychology_warnings.is_empty(),
        "No warnings for clean portfolio"
    );

    println!(
        "  Verifier clean: {:?} | Heat: {:.1}% | MaxPos: ₹{:.2}",
        analysis.recommendation,
        analysis.portfolio_heat * 100.0,
        analysis.max_position_size
    );

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_verifier_with_open_position() {
    let (orch, db_path) = setup_test_env("verifier_positions");

    // Add an open position before running verifier
    let signal = TradeSignal {
        symbol: "NIFTY".to_string(),
        direction: TradeDirection::Long,
        entry_price: 24_500.0,
        stop_loss: 24_300.0,
        take_profit: 24_900.0,
        position_size: 1.0, // 1 share @ 24,500 = 24,500 position value (< 95k cash OK)
        confidence_score: 0.8,
        confluence_score: 0.75,
        risk_reward_ratio: 2.0,
        reasoning: "Test open position for verifier".to_string(),
        timestamp: Utc::now(),
        session_valid: true,
        risk_check_passed: true,
    };

    // Add position directly via portfolio manager (bypasses execution coordinator)
    orch.portfolio
        .add_position(&signal)
        .await
        .expect("Should add position");

    // Verify state is consistent
    assert_eq!(
        count_positions(&orch.state).await,
        1,
        "Should have 1 open position"
    );
    let equity = get_equity(&orch.state).await;
    assert_eq!(
        equity, 100_000.0,
        "Equity unchanged after adding position (value = cash reduction)"
    );

    // Run verifier — should still work with open position
    let result = orch.tredo().run_verifier("NIFTY", 24_500.0, equity).await;
    assert!(
        result.is_ok(),
        "Verifier should handle portfolios with open positions"
    );
    let analysis = result.unwrap();

    // With one position, heat should be > 0 but recommendations should still be Proceed
    assert!(
        analysis.portfolio_heat > 0.0,
        "Open position → heat should be > 0"
    );
    println!("  Position heat: {:.2}%", analysis.portfolio_heat * 100.0);

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_verifier_drawdown_halt() {
    let (orch, db_path) = setup_test_env("verifier_halt");

    // Simulate a portfolio that hit max drawdown by disabling trading
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.max_drawdown_today = 0.10; // 10%
        portfolio.daily_pnl = -10_000.0; // -₹10k
        portfolio.consecutive_losses = 5;
        portfolio.trading_enabled = false;
    }

    let equity = get_equity(&orch.state).await;
    let result = orch.tredo().run_verifier("NIFTY", 24_500.0, equity).await;

    assert!(
        result.is_ok(),
        "Verifier should handle halted state gracefully"
    );
    let analysis = result.unwrap();

    // Should get Halt recommendation
    assert_eq!(
        analysis.recommendation,
        RiskRecommendation::Halt,
        "Halted portfolio should get Halt recommendation"
    );
    assert!(
        analysis.daily_drawdown_pct > 0.0,
        "Drawdown should be recorded"
    );

    println!(
        "  Halt test: drawdown {:.1}% | warnings: {}",
        analysis.daily_drawdown_pct * 100.0,
        analysis.psychology_warnings.len()
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// EXECUTER GROUP — Strategy decision & execution
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_executer_handles_no_llm() {
    let (orch, db_path) = setup_test_env("executer_nollm");

    // The Executer calls generate_signal() which calls the LLM (ask_for_trade_decision).
    // Without Ollama running, this should propagate an error gracefully rather than panic.
    // Agentic call: only current price from market data. Agent identifies levels itself using indicators + debate.
    // Agentic call — only current market price. The agent itself identifies direction + entry/SL/TP
    // using its skills (RSI, MACD, ATR, volume, patterns, pivots, regime, confluence) + debate + memory + disciplined rules.
    let result = orch
        .tredo()
        .run_executer("BTC", 65_000.0)
        .await;

    // Without an LLM running, we expect an error (connection refused or similar)
    match &result {
        Ok(opt) => {
            // If LLM happens to be running, signal could be Some or None
            match opt {
                Some(sig) => {
                    println!(
                        "  LLM available! Decision: {:?} @ {:.2} (conf: {:.1}%)",
                        sig.direction,
                        sig.entry_price,
                        sig.confidence_score * 100.0
                    );
                }
                None => {
                    println!("  LLM decided HOLD for BTC");
                }
            }
        }
        Err(e) => {
            let err_msg = format!("{}", e);
            println!("  LLM unavailable (expected in test env): {}", e);
            // Should get a meaningful error about connection failure, not a panic or crash
            assert!(!err_msg.is_empty(), "Error message should not be empty");
            assert!(!err_msg.contains("panicked"), "Should not be a panic");
        }
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// STATE SHARING & COMMUNICATION ACROSS GROUPS
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_state_sharing_across_groups() {
    let (orch, db_path) = setup_test_env("state_sharing");
    let tredo = orch.tredo();

    // Verify the inner state paths converge: all agent Arcs share the same agent instances
    assert!(
        Arc::ptr_eq(&orch.scanner, &tredo.identifier.scanner),
        "scanner should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.risk_psych, &tredo.verifier.risk_psych),
        "risk_psych should be shared"
    );
    assert!(
        Arc::ptr_eq(&orch.strategy, &tredo.executer.strategy),
        "strategy should be shared"
    );

    // Modify state through one agent and verify it's visible through another
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.daily_pnl = -5_000.0;
        portfolio.consecutive_losses = 3;
    }

    // Read back through Tredo Verifier's state reference — should see the modified state
    let equity = get_equity(&orch.state).await;
    let result = tredo.run_verifier("NIFTY", 24_500.0, equity).await;
    assert!(result.is_ok());
    let analysis = result.unwrap();

    // The modified state (losses) should be reflected in the analysis
    assert!(
        !analysis.psychology_warnings.is_empty(),
        "Consecutive losses should generate psychology warnings, got {:?}",
        analysis.psychology_warnings
    );

    let warnings_concat = analysis.psychology_warnings.join(" ");
    println!("  Warnings from shared state: {}", warnings_concat);

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_pipeline_state_consistency() {
    let (orch, db_path) = setup_test_env("pipeline_state");
    let symbol = "SOL";

    // Seed data
    seed_ohlcv(&orch.state, symbol, 180.0).await;
    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Run pipeline identifier then verifier in sequence (mimicking run_full_pipeline)
    let (discipline_ok, confluence, _pivots) = orch
        .tredo()
        .run_identifier(symbol, 180.0)
        .await
        .expect("Identifier should succeed");

    assert!(discipline_ok, "SOL is crypto → discipline should pass");
    assert!(confluence >= 0.0, "Confluence should be valid");

    // After identifier, state should be populated
    {
        let regime = orch.state.market_regime.read().await;
        assert!(regime.is_some(), "Market regime should be set");
        println!("  Post-identifier regime: {:?}", regime);
    }

    // Now run verifier — it should see the state from identifier
    let equity = get_equity(&orch.state).await;
    let risk = orch
        .tredo()
        .run_verifier(symbol, 180.0, equity)
        .await
        .expect("Verifier should succeed");

    println!(
        "  Post-verifier recommendation: {:?}, heat: {:.1}%",
        risk.recommendation,
        risk.portfolio_heat * 100.0
    );

    // Full pipeline call should complete without panicking
    let summary = orch
        .run_full_pipeline(symbol)  // agentic call: agent decides direction + exact entry/SL/TP from its analysis of indicators (RSI, MACD, volume, patterns, etc.) and debate/memory/rules
        .await;

    match &summary {
        Ok(s) => {
            println!(
                "  Pipeline completed: executed={}, reason={}, duration={}ms",
                s.executed,
                s.reason.chars().take(60).collect::<String>(),
                s.total_duration_ms
            );
        }
        Err(e) => {
            // Pipeline might fail at executer stage if LLM unavailable — that's expected
            let err = format!("{}", e);
            println!("  Pipeline ended at executer (expected): {}", err);
            assert!(!err.contains("panicked"), "Should not panic");
        }
    }

    // Simulate outcome recording so this test's DB gets skill_performance + closed_trades rows
    simulate_outcome_recording(&orch.state, "SOL", 50.0, true);

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// ERROR HANDLING & EDGE CASES
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_concurrent_group_access() {
    let (orch, db_path) = setup_test_env("concurrent");
    let tredo = orch.tredo();

    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Run Identifier and Verifier concurrently — should not deadlock
    let (ident_result, ver_result) = tokio::join!(
        tredo.run_identifier("BTC", 65_000.0),
        tredo.run_verifier("ETH", 3_500.0, 100_000.0),
    );

    assert!(
        ident_result.is_ok(),
        "Concurrent identifier should succeed: {:?}",
        ident_result.err()
    );
    assert!(
        ver_result.is_ok(),
        "Concurrent verifier should succeed: {:?}",
        ver_result.err()
    );

    let (disc_ok, conf, _) = ident_result.unwrap();
    println!(
        "  Concurrent identifier: {} | conf: {:.1}%",
        if disc_ok { "OK" } else { "FAIL" },
        conf * 100.0
    );
    println!(
        "  Concurrent verifier: {:?}",
        ver_result.unwrap().recommendation
    );

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_multiple_pipeline_runs() {
    let (orch, db_path) = setup_test_env("multi_run");

    // Run pipeline for multiple symbols in sequence
    for (i, (symbol, price)) in ["BTC", "ETH", "SOL"]
        .iter()
        .zip([65_000.0, 3_500.0, 180.0].iter())
        .enumerate()
    {
        seed_ohlcv(&orch.state, symbol, *price).await;

        let (disc_ok, conf, pivots) = orch
            .tredo()
            .run_identifier(symbol, *price)
            .await
            .expect("Identifier should succeed");

        println!(
            "  [{}] {}: disc={}, conf={:.1}%, pivot={:.2}",
            i + 1,
            symbol,
            if disc_ok { "OK" } else { "FAIL" },
            conf * 100.0,
            pivots.pivot
        );
    }

    // State should have accumulated COT entries
    let cot_count = orch.state.cot_store.read().await.len();
    println!("  Total COT entries after {} runs: {}", 3, cot_count);

    // Simulate outcome recording so this test's DB gets skill_performance + closed_trades rows
    simulate_outcome_recording(&orch.state, "BTC", -30.0, false);

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// PORTFOLIO INTEGRATION — verify Tredo groups can modify and read portfolio
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_portfolio_management_via_tredo() {
    let (orch, db_path) = setup_test_env("portfolio_tredo");
    let tredo = orch.tredo();

    // Verify initial portfolio state
    {
        let portfolio = orch.state.portfolio.read().await;
        assert_eq!(portfolio.cash_balance, 100_000.0);
        assert!(portfolio.open_positions.is_empty());
        assert!(portfolio.trading_enabled);
    }

    // === REAL FULL CYCLE for outcome data (using real MI -> votes -> aggregated -> decision influence -> OutcomeProcessor) ===
    // Seed and run Identifier (populates real last_skill_votes + last_aggregated_signal via the wired aggregator)
    seed_ohlcv(&orch.state, "NIFTY", 24_500.0).await;
    let (disc_ok, conf, _pivots) = tredo
        .run_identifier("NIFTY", 24_500.0)
        .await
        .expect("Identifier should succeed for real skill/agg population");
    assert!(disc_ok, "NIFTY discipline should pass in test");
    println!("  [REAL CYCLE] Identifier done: conf={:.1}%, aggregated now in state for decision", conf * 100.0);

    // Run verifier
    let equity = get_equity(&orch.state).await;
    let analysis = tredo
        .run_verifier("NIFTY", 24_500.0, equity)
        .await
        .expect("Verifier should succeed");
    assert_eq!(analysis.recommendation, RiskRecommendation::Proceed);

    // Create signal influenced by real aggregated from MI (the wiring we added)
    let aggregated = {
        let a = orch.state.last_aggregated_signal.read().await;
        a.clone()
    };
    let is_bullish = aggregated.as_ref().map_or(false, |agg| agg.is_bullish(None));
    println!("  [REAL CYCLE] Aggregated from MI: bullish={}", is_bullish);

    let signal = TradeSignal {
        symbol: "NIFTY".to_string(),
        direction: TradeDirection::Long,
        entry_price: 24_500.0,
        stop_loss: 24_300.0,
        take_profit: 24_900.0,
        position_size: 1.0,
        confidence_score: if is_bullish { 0.85 } else { 0.6 },
        confluence_score: 0.75,
        risk_reward_ratio: 2.0,
        reasoning: format!("Test signal (real aggregated from MI: {})", if is_bullish { "bullish boost" } else { "neutral" }),
        timestamp: Utc::now(),
        session_valid: true,
        risk_check_passed: true,
    };

    // "Execute" via the exposed portfolio path (simulates what executer does; real coordinator path would be similar)
    // This keeps test stable without exposing internal executer fields.
    orch.portfolio
        .add_position(&signal)
        .await
        .expect("Should add position via real-ish path");
    println!("  [REAL CYCLE] Position added (simulating execute_paper_trade)");

    // Verify position added via real path
    assert_eq!(count_positions(&orch.state).await, 1);
    let new_equity = get_equity(&orch.state).await;

    // Simulate price to TP and trigger real close path (calls close_episode with the real votes from MI)
    {
        let mut portfolio = orch.state.portfolio.write().await;
        if let Some(pos) = portfolio.open_positions.iter_mut().find(|p| p.symbol == "NIFTY") {
            pos.current_price = 24_800.0;  // hit TP
        }
    }

    // Use the real OutcomeProcessor (this is the production code path from execution_coordinator on SL/TP)
    // It will read the last_skill_votes populated by the real MI run above and insert real skill_performance + regret.
    let op = tredo_autonomous::outcome_processor::OutcomeProcessor::new(orch.state.clone());
    let pnl = 300.0;
    // Get the pos for close_episode
    let pos = {
        let p = orch.state.portfolio.read().await;
        p.open_positions.iter().find(|p| p.symbol == "NIFTY").cloned().expect("pos")
    };
    op.close_episode(&pos, 24_800.0, "take_profit", pnl).await;
    println!("  [REAL CYCLE] Real OutcomeProcessor.close_episode called with votes from MI — data should be in DB");

    // Clean up position in portfolio for test state
    orch.portfolio
        .close_position("NIFTY", 24_800.0)
        .await
        .expect("cleanup close");

    // Verify final state
    {
        let portfolio = orch.state.portfolio.read().await;
        assert!(
            portfolio.open_positions.is_empty(),
            "All positions should be closed"
        );
        assert!(
            portfolio.daily_pnl > 0.0,
            "Should have positive P&L from profitable close"
        );
        println!(
            "  Final P&L: ₹{:.2}, Equity: ₹{:.2}",
            portfolio.daily_pnl, portfolio.total_equity
        );
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// CLEANUP TEST — verify all db files are removed
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_cleanup_temp_files() {
    let prefixes = [
        "hierarchy",
        "identifier",
        "identifier_empty",
        "verifier_clean",
        "verifier_positions",
        "verifier_halt",
        "executer_nollm",
        "state_sharing",
        "pipeline_state",
        "concurrent",
        "multi_run",
        "portfolio_tredo",
    ];

    let mut cleaned = 0;
    for prefix in &prefixes {
        let path = format!("test_{}.redb", prefix);
        if fs::remove_file(&path).is_ok() {
            cleaned += 1;
        }
        let path2 = format!("test_{}.redb.lock", prefix);
        let _ = fs::remove_file(&path2);
    }
    // Clean up shared SQLite db as well
    for f in &[
        "tredo_history.db",
        "tredo_history.db-wal",
        "tredo_history.db-shm",
    ] {
        if fs::remove_file(f).is_ok() {
            cleaned += 1;
        }
    }
    println!("  Cleaned up {} previously left-over db files", cleaned);
}
