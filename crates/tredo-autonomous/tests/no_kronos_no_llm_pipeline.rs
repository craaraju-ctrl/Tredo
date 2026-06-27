// no_kronos_no_llm_pipeline.rs
// Integration test: runs the full autonomous pipeline with both Kronos and LLM
// permanently disconnected. Verifies that the rule-based fallback path:
//   1. Does not panic or return fatal errors at any agent layer
//   2. Produces a valid signal (BUY/SELL) OR a clean HOLD — never garbage
//   3. All 16 sub-agents across 4 groups execute without crashing
//   4. The pipeline summary is well-formed
//
// This is the resilience test the user requested after discovering the system
// was effectively running on rule-based signals only, with both Kronos and LLM
// orphaned from the decision path.
//
// Run: cargo test -p tredo-autonomous --test no_kronos_no_llm_pipeline

use chrono::Utc;
use std::fs;
use tredo_autonomous::{AutonomousOrchestrator, SharedState};
use tredo_core::{Config, DisciplineRules, MemoryStore, OhlcvBar, TradeDirection};

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn setup_no_external_deps(db_name: &str) -> (AutonomousOrchestrator, String) {
    let sqlite_db_path = format!("test_noext_{}.db", db_name);
    for f in &[
        sqlite_db_path.to_string(),
        format!("{}-wal", sqlite_db_path),
        format!("{}-shm", sqlite_db_path),
    ] {
        let _ = fs::remove_file(f);
    }
    let db_path = format!("test_noext_{}.redb", db_name);
    let _ = fs::remove_file(&db_path);

    let memory = MemoryStore::new(&db_path).expect("MemoryStore creation");
    // Pre-configure Kronos to an unreachable port so it fails immediately.
    let config = Config {
        kronos_service_url: "http://127.0.0.1:19999".to_string(),
        ..Config::default()
    };
    let rules = DisciplineRules::default();
    let state = SharedState::new(memory, rules, config, &sqlite_db_path).expect("SharedState init");
    // Clear calendar events so red_folder Critical rule doesn't block pipeline
    *state.calendar_events.write().await = Vec::new();

    let mut orch = AutonomousOrchestrator::new(state);
    orch.init_tredo();
    (orch, db_path)
}

/// Seed a high-conviction aggregated signal so the HardRulesGate confluence
/// check passes (default regime "None" needs ≥65% confluence).
async fn seed_aggregated_signal(state: &SharedState, symbol: &str) {
    use tredo_core::agent::SkillDirection;
    use tredo_core::skill_aggregator::AggregatedSignal;
    let agg = AggregatedSignal {
        net_signal: 0.6,
        bullish_strength: 0.7,
        bearish_strength: 0.1,
        conviction: 0.75, // well above 65% minimum
        consensus: Some(SkillDirection::Bullish),
        participating_count: 5,
        bullish_count: 4,
        bearish_count: 1,
        neutral_count: 0,
    };
    let mut last_agg = state.last_aggregated_signal.write().await;
    *last_agg = Some(agg);
    println!(
        "  📊 Seeded aggregated signal for {} with conviction=0.75",
        symbol
    );
}

/// Seed realistic OHLCV data with enough bars for RSI/MACD/ATR computation.
async fn seed_rich_ohlcv(state: &SharedState, symbol: &str, base_price: f64) {
    let mut history = state.ohlcv_history.write().await;
    let mut bars = Vec::with_capacity(50);
    for i in 0..50 {
        // Create a trending pattern: base_price + sinusoidal trend + noise
        let trend = base_price * (i as f64 * 0.005); // slow uptrend
        let noise = (i as f64 * 0.7).sin() * base_price * 0.008;
        let close = base_price + trend + noise;
        let high = close + (base_price * 0.003);
        let low = close - (base_price * 0.003);
        let open = close - (noise * 0.5);
        bars.push(OhlcvBar {
            timestamp: (Utc::now() - chrono::Duration::minutes(50 - i as i64)).to_rfc3339(),
            open,
            high,
            low,
            close,
            volume: 150_000.0 + (i as f64) * 2_000.0,
        });
    }
    history.insert(symbol.to_string(), bars);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 1: Full pipeline completes without panic when Kronos + LLM are both off
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_full_pipeline_no_kronos_no_llm() {
    let (orch, db_path) = setup_no_external_deps("pipeline_noext").await;

    // Seed rich OHLCV so indicators (RSI, MACD, ATR, patterns) have data
    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_rich_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Seed aggregated signal so HardRulesGate confluence check passes
    seed_aggregated_signal(&orch.state, "BTC").await;

    // Run the full pipeline — this exercises all 16 sub-agents
    // Kronos: unreachable (127.0.0.1:19999), LLM: Ollama not running
    let result = orch.run_full_pipeline("BTC").await;

    // The pipeline MUST NOT panic or return an error
    // It can return PipelineSummary with executed=false (HOLD) — that's fine
    match &result {
        Ok(summary) => {
            println!(
                "  ✅ Pipeline completed: executed={} reason=\"{}\" duration={}ms",
                summary.executed, summary.reason, summary.total_duration_ms
            );

            // Verify the summary is well-formed
            assert!(
                summary.total_duration_ms > 0,
                "Pipeline should take measurable time"
            );
            assert!(!summary.reason.is_empty(), "Pipeline should have a reason");

            // Verify the signal (if any) is well-formed
            if let Some(signal) = &summary.final_signal {
                assert_eq!(signal.symbol, "BTC");
                assert!(
                    signal.direction == TradeDirection::Long
                        || signal.direction == TradeDirection::Short,
                    "Signal direction should be Long or Short, got {:?}",
                    signal.direction
                );
                assert!(signal.entry_price > 0.0, "Entry price should be positive");
                assert!(signal.stop_loss > 0.0, "Stop loss should be positive");
                assert!(signal.take_profit > 0.0, "Take profit should be positive");
                assert!(
                    signal.position_size > 0.0,
                    "Position size should be positive"
                );
                println!(
                    "  📊 Signal: {:?} entry={:.2} SL={:.2} TP={:.2} size={:.2} conf={:.1}%",
                    signal.direction,
                    signal.entry_price,
                    signal.stop_loss,
                    signal.take_profit,
                    signal.position_size,
                    signal.confidence_score * 100.0
                );
            } else {
                println!("  📊 Pipeline returned HOLD (no signal) — valid rule-based outcome");
            }
        }
        Err(e) => {
            panic!(
                "Full pipeline MUST NOT error when Kronos+LLM are off. Got: {}",
                e
            );
        }
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 2: Identifier group completes with Kronos failing
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_identifier_with_kronos_down() {
    let (orch, db_path) = setup_no_external_deps("ident_kronos_down").await;
    seed_rich_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Run just the Identifier group
    let result = orch.tredo().run_identifier("ETH", 3_500.0, 1).await;

    assert!(
        result.is_ok(),
        "Identifier must complete without error when Kronos is down: {:?}",
        result.err()
    );

    let (discipline_ok, confluence, pivots) = result.unwrap();

    // Confluence should be a valid score (0-1 range)
    assert!(
        (0.0..=1.0).contains(&confluence),
        "Confluence should be in [0,1], got {}",
        confluence
    );

    // Pivots should be finite and reasonable
    assert!(pivots.pivot.is_finite(), "Pivot should be finite");
    assert!(pivots.r1.is_finite(), "R1 should be finite");
    assert!(pivots.s1.is_finite(), "S1 should be finite");

    // ETH is crypto → session check should pass
    assert!(discipline_ok, "Crypto should pass discipline checks");

    println!(
        "  ✅ Identifier: discipline={}, confluence={:.1}%, pivot={:.2}",
        discipline_ok,
        confluence * 100.0,
        pivots.pivot
    );

    // Verify Kronos forecast was stored as None (unavailable)
    let forecast = orch.state.last_forecast.read().await;
    assert!(
        forecast.is_none(),
        "Kronos forecast should be None when service is unreachable"
    );
    println!("  📊 Kronos forecast: None (as expected)");

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 3: Strategy decision produces valid output when both Kronos + LLM are off
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_strategy_decision_no_external_deps() {
    let (orch, db_path) = setup_no_external_deps("strat_noext").await;
    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_rich_ohlcv(&orch.state, "NIFTY", 24_500.0).await;

    // Run identifier first to populate state (MI skills, patterns, regime, etc.)
    orch.tredo()
        .run_identifier("BTC", 65_000.0, 1)
        .await
        .expect("Identifier should succeed");

    // Now run the strategy decision directly
    let signal = orch.strategy.generate_signal("BTC", 65_000.0).await;

    assert!(
        signal.is_ok(),
        "Strategy decision must not error: {:?}",
        signal.err()
    );

    match signal.unwrap() {
        Some(sig) => {
            // If a signal is produced, validate its structure
            assert_eq!(sig.symbol, "BTC");
            assert!(
                sig.entry_price > 0.0 && sig.entry_price.is_finite(),
                "Entry must be positive and finite"
            );
            assert!(
                sig.stop_loss > 0.0 && sig.stop_loss.is_finite(),
                "SL must be positive and finite"
            );
            assert!(
                sig.take_profit > 0.0 && sig.take_profit.is_finite(),
                "TP must be positive and finite"
            );
            assert!(sig.position_size > 0.0, "Position size must be positive");
            assert!(
                sig.confidence_score > 0.0 && sig.confidence_score <= 1.0,
                "Confidence must be in (0, 1], got {}",
                sig.confidence_score
            );
            assert!(!sig.reasoning.is_empty(), "Reasoning should not be empty");

            // Verify SL/TP geometry makes sense
            match sig.direction {
                TradeDirection::Long => {
                    assert!(
                        sig.stop_loss < sig.entry_price,
                        "Long SL ({}) should be below entry ({})",
                        sig.stop_loss,
                        sig.entry_price
                    );
                    assert!(
                        sig.take_profit > sig.entry_price,
                        "Long TP ({}) should be above entry ({})",
                        sig.take_profit,
                        sig.entry_price
                    );
                }
                TradeDirection::Short => {
                    assert!(
                        sig.stop_loss > sig.entry_price,
                        "Short SL ({}) should be above entry ({})",
                        sig.stop_loss,
                        sig.entry_price
                    );
                    assert!(
                        sig.take_profit < sig.entry_price,
                        "Short TP ({}) should be below entry ({})",
                        sig.take_profit,
                        sig.entry_price
                    );
                }
            }

            println!(
                "  ✅ Strategy produced signal: {:?} @ {:.2} (conf {:.1}%, RR {:.1}:1)",
                sig.direction,
                sig.entry_price,
                sig.confidence_score * 100.0,
                sig.risk_reward_ratio
            );
            println!(
                "  📝 Reasoning: {}",
                sig.reasoning.chars().take(120).collect::<String>()
            );
        }
        None => {
            println!("  ✅ Strategy decided HOLD — valid rule-based outcome when no external deps");
        }
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 4: Debate agents produce valid votes when LLM is unavailable
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_debate_agents_without_llm() {
    let (orch, db_path) = setup_no_external_deps("debate_noext").await;
    seed_rich_ohlcv(&orch.state, "SOL", 180.0).await;

    let input = tredo_core::AgentInput::ConfluenceRequest {
        context: tredo_core::MarketContext {
            symbol: "SOL".to_string(),
            current_price: 180.0,
            high: 183.0,
            low: 177.0,
            previous_close: 179.0,
            timestamp: Utc::now(),
            daily_pnl: 0.0,
            equity: 100_000.0,
            consecutive_losses: 0,
            is_red_folder_day: false,
            trend_direction: None,
        },
    };

    // Run debate with no LLM — all 4 agents should produce valid votes
    let (action, conf, reason, turns) = tredo_autonomous::debate::run_debate(
        orch.state.clone(),
        &input,
        None, // no aggregated signal
    )
    .await;

    // Validate debate output
    assert!(
        action == "BUY" || action == "SELL" || action == "HOLD",
        "Debate action should be BUY/SELL/HOLD, got '{}'",
        action
    );
    assert!(
        (0.0..=1.0).contains(&conf),
        "Debate confidence should be in [0,1], got {}",
        conf
    );
    assert!(!reason.is_empty(), "Debate reasoning should not be empty");
    assert_eq!(
        turns.len(),
        4,
        "Debate should have exactly 4 turns (prop/crit/risk/hist)"
    );

    // Each turn should have a valid action
    for turn in &turns {
        assert!(!turn.action.is_empty(), "Turn action should not be empty");
        assert!(
            turn.confidence >= 0.0 && turn.confidence <= 1.0,
            "Turn confidence should be in [0,1], got {}",
            turn.confidence
        );
    }

    println!(
        "  ✅ Debate: action={} conf={:.2} turns={}",
        action,
        conf,
        turns.len()
    );
    for (i, turn) in turns.iter().enumerate() {
        println!(
            "    Turn {}: {} (conf {:.2}) — {}",
            i + 1,
            turn.action,
            turn.confidence,
            turn.reasoning.chars().take(80).collect::<String>()
        );
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 5: Multiple pipeline runs — verify no state corruption over iterations
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_multiple_iterations_no_degradation() {
    let (orch, db_path) = setup_no_external_deps("multi_iter").await;
    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;

    // Seed aggregated signal so HardRulesGate confluence check passes
    seed_aggregated_signal(&orch.state, "BTC").await;

    let mut signal_count = 0;
    let mut hold_count = 0;

    for i in 0..5 {
        let result = orch.run_full_pipeline("BTC").await;
        assert!(
            result.is_ok(),
            "Pipeline iteration {} must not error: {:?}",
            i + 1,
            result.err()
        );

        let summary = result.unwrap();
        if summary.executed {
            signal_count += 1;
        } else {
            hold_count += 1;
        }

        println!(
            "  Iteration {}: executed={} reason=\"{}\" duration={}ms",
            i + 1,
            summary.executed,
            summary.reason,
            summary.total_duration_ms
        );
    }

    println!(
        "  📊 Over 5 iterations: {} signals, {} holds",
        signal_count, hold_count
    );

    // Every iteration must complete successfully (asserted above via assert!(result.is_ok()))
    // Signals vs holds depends on debate scoring — both are valid outcomes.
    // The key assertion is that NO iteration panicked or returned an error.
    println!(
        "  ✅ All 5 iterations completed without errors. Signals: {}, Holds: {}",
        signal_count, hold_count
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 6: All 16 sub-agents execute without crash via Identifier + Verifier
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_all_16_agents_execute() {
    let (orch, db_path) = setup_no_external_deps("all_agents").await;
    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_rich_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Seed aggregated signal so HardRulesGate confluence check passes
    seed_aggregated_signal(&orch.state, "BTC").await;

    // Run Identifier (7 agents: scanner, MI, pivot, confluence, patterns, session, red_folder)
    let ident_result = orch.tredo().run_identifier("BTC", 65_000.0, 1).await;
    assert!(
        ident_result.is_ok(),
        "Identifier (7 agents) must not crash: {:?}",
        ident_result.err()
    );
    println!("  ✅ Identifier group (7 agents) completed");

    // Run Verifier (3 agents: risk_psych, risk_calc, reflector + 2 guardian: drawdown, overtrading)
    let equity = orch.state.portfolio.read().await.total_equity;
    let verifier_result = orch.tredo().run_verifier("BTC", 65_000.0, equity, 2).await;
    assert!(
        verifier_result.is_ok(),
        "Verifier (3+2 agents) must not crash: {:?}",
        verifier_result.err()
    );
    println!("  ✅ Verifier group (3+2 agents) completed");

    // Run Executer (3 agents: strategy, portfolio, execution)
    let executer_result = orch.tredo().run_executer("BTC", 65_000.0).await;
    assert!(
        executer_result.is_ok(),
        "Executer (3 agents) must not crash: {:?}",
        executer_result.err()
    );
    println!("  ✅ Executer group (3 agents) completed");

    // COT store should have entries from agents recording their work
    let cot_count = orch.state.cot_store.read().await.len();
    assert!(
        cot_count > 0,
        "Should have COT entries from agents, got {}",
        cot_count
    );
    println!(
        "  📊 COT entries: {} (agents recording pipeline progress)",
        cot_count
    );

    let _ = fs::remove_file(&db_path);
}
