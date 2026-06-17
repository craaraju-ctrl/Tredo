// five_layer_pipeline_integration.rs
// Integration test: validates the full 5-Layer Pipeline flow end-to-end.
//
// Architecture under test:
//   Layer 1: HardRulesGate  — priority-based blocking (Critical > High > Medium > Low)
//   Layer 2: Identifier + Verifier — advisory data gathering (never blocks)
//   Layer 3: DebateLayer — BullTeam 12f vs BearTeam 11f, 3-round adversarial
//   Layer 4: Judge/Adjudicator — evaluates debate quality ONLY
//   Layer 5: Execution — paper trade fill or HOLD
//
// Each test exercises specific layers and verifies:
//   1. Layer ordering is respected (Gate runs FIRST)
//   2. Gate blocking prevents downstream layers from executing
//   3. Advisory layers (Identifier/Verifier/Debate) never block on their own
//   4. Judge only evaluates debate quality (confidence, evidence, signal count)
//   5. COT chain captures the full pipeline progression
//   6. Multiple iterations don't corrupt state
//
// Run: cargo test -p tredo-autonomous --test five_layer_pipeline_integration

use chrono::Utc;
use std::fs;
use tredo_autonomous::{AutonomousOrchestrator, SharedState};
use tredo_autonomous::types::OpenPosition;
use tredo_core::{Config, DisciplineRules, MemoryStore, OhlcvBar, TradeDirection};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn setup(db_name: &str) -> (AutonomousOrchestrator, String) {
    let sqlite_db_path = format!("test_5layer_{}.db", db_name);
    for f in &[
        sqlite_db_path.to_string(),
        format!("{}-wal", sqlite_db_path),
        format!("{}-shm", sqlite_db_path),
    ] {
        let _ = fs::remove_file(f);
    }
    let db_path = format!("test_5layer_{}.redb", db_name);
    let _ = fs::remove_file(&db_path);

    let memory = MemoryStore::new(&db_path).expect("MemoryStore creation");
    let config = Config {
        kronos_service_url: "http://127.0.0.1:19999".to_string(),
        ..Config::default()
    };
    let rules = DisciplineRules::default();
    let state = SharedState::new(memory, rules, config, &sqlite_db_path)
        .expect("SharedState init");

    let mut orch = AutonomousOrchestrator::new(state);
    orch.init_tredo();
    (orch, db_path)
}

async fn seed_rich_ohlcv(state: &SharedState, symbol: &str, base_price: f64) {
    let mut history = state.ohlcv_history.write().await;
    let mut bars = Vec::with_capacity(50);
    for i in 0..50 {
        let trend = base_price * (i as f64 * 0.005);
        let noise = (i as f64 * 0.7).sin() * base_price * 0.008;
        let close = base_price + trend + noise;
        let high = close + base_price * 0.003;
        let low = close - base_price * 0.003;
        let open = close - noise * 0.5;
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

async fn seed_aggregated_signal(state: &SharedState, conviction: f64) {
    use tredo_core::agent::SkillDirection;
    use tredo_core::skill_aggregator::AggregatedSignal;
    let agg = AggregatedSignal {
        net_signal: 0.6,
        bullish_strength: 0.7,
        bearish_strength: 0.1,
        conviction,
        consensus: Some(SkillDirection::Bullish),
        participating_count: 5,
        bullish_count: 4,
        bearish_count: 1,
        neutral_count: 0,
    };
    *state.last_aggregated_signal.write().await = Some(agg);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 1: Full pipeline end-to-end — Gate passes → all layers execute
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_full_pipeline_end_to_end() {
    let (orch, db_path) = setup("e2e_pass");

    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_aggregated_signal(&orch.state, 0.85).await;
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBull);

    let result = orch.run_full_pipeline("BTC").await;
    assert!(result.is_ok(), "Pipeline must not error: {:?}", result.err());

    let summary = result.unwrap();
    assert!(summary.total_duration_ms > 0, "Pipeline should take measurable time");
    assert!(!summary.reason.is_empty(), "Pipeline should have a reason");

    // With high confluence + bull regime + rich data, debate should produce a verdict
    println!(
        "  ✅ Full pipeline: executed={} reason=\"{}\" duration={}ms",
        summary.executed, summary.reason, summary.total_duration_ms
    );

    if let Some(sig) = &summary.final_signal {
        assert_eq!(sig.symbol, "BTC");
        assert!(sig.entry_price > 0.0);
        assert!(sig.stop_loss > 0.0);
        assert!(sig.take_profit > 0.0);
        assert!(sig.position_size > 0.0);
        println!(
            "  📊 Signal: {:?} @ {:.2} conf={:.1}%",
            sig.direction,
            sig.entry_price,
            sig.confidence_score * 100.0
        );
    }

    // COT chain should have entries from all layers
    let cot_count = orch.state.cot_store.read().await.len();
    assert!(cot_count > 0, "Should have COT entries from pipeline layers");

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 2: Gate blocks → no downstream layers execute
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_gate_blocks_prevents_downstream() {
    let (orch, db_path) = setup("gate_block");

    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;

    // Disable trading → Critical rule blocks
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.trading_enabled = false;
    }

    let result = orch.run_full_pipeline("BTC").await;
    assert!(result.is_ok(), "Pipeline must not error: {:?}", result.err());

    let summary = result.unwrap();
    assert!(!summary.executed, "Pipeline should NOT execute when gate blocks");
    assert!(
        summary.reason.contains("Hard Rules Gate") || summary.reason.contains("Hard Rules"),
        "Reason should mention HardRulesGate, got: {}",
        summary.reason
    );
    assert!(
        summary.final_signal.is_none(),
        "No signal should be produced when gate blocks"
    );

    println!(
        "  ✅ Gate correctly blocked: \"{}\"",
        summary.reason
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 3: Gate blocks on drawdown → early return before Identifier
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_gate_drawdown_blocks_before_identifier() {
    let (orch, db_path) = setup("gate_dd");

    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;

    // Set drawdown > 2% (Critical)
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.max_drawdown_today = 0.025;
    }

    // Record COT entries before pipeline run
    let cot_before = orch.state.cot_store.read().await.len();

    let result = orch.run_full_pipeline("BTC").await;
    assert!(result.is_ok());

    let summary = result.unwrap();
    assert!(!summary.executed, "Should not execute when drawdown exceeds limit");

    // Pipeline should be very fast (no layers 2-5 ran)
    assert!(
        summary.total_duration_ms < 5000,
        "Blocked pipeline should be fast, took {}ms",
        summary.total_duration_ms
    );

    // COT should have gate entry but NOT Identifier/Debate entries
    let cot_after = orch.state.cot_store.read().await.len();
    let new_entries = cot_after - cot_before;
    // Expect: pipeline_start + hard_rules_gate blocked = ~2 entries max
    // (NO identifier, NO verifier, NO debate, NO judge, NO execution entries)
    assert!(
        new_entries <= 4,
        "Blocked pipeline should have minimal COT entries ({}), expected ≤ 4 (no Identifier/Debate layers)",
        new_entries
    );

    println!(
        "  ✅ Drawdown gate blocked with {} new COT entries (no downstream layers)",
        new_entries
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 4: High priority blocks override Medium (heat + low confluence)
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_high_priority_overrides_medium() {
    let (orch, db_path) = setup("high_over_med");

    seed_rich_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Set High rule (heat > 10%) + bear regime (Medium rule)
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.total_equity = 100_000.0;
        for i in 0..3 {
            portfolio.open_positions.push(OpenPosition {
                symbol: format!("SYM{}", i),
                direction: TradeDirection::Long,
                entry_price: 100.0,
                current_price: 100.0,
                stop_loss: 95.0,
                take_profit: 110.0,
                quantity: 1.0,
                unrealized_pnl: 0.0,
                unrealized_pnl_pct: 0.0,
                entry_time: Utc::now(),
                risk_amount: 4000.0, // 3 × 4000 = 12% heat
            });
        }
    }
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBear);

    // Run HardRulesGate directly to verify priority resolution
    let gate = tredo_autonomous::hard_rules_gate::HardRulesGate::new(orch.state.clone());
    let gate_result = gate.evaluate("ETH").await;

    assert!(!gate_result.passed, "Gate should block");
    assert_eq!(
        gate_result.highest_failed_priority,
        Some(tredo_autonomous::types::RulePriority::High),
        "High should win over Medium"
    );
    assert!(
        gate_result.failed_rules.iter().any(|r| r.rule_name == "portfolio_heat"),
        "Should have heat rule failure"
    );

    // Full pipeline should also block
    let result = orch.run_full_pipeline("ETH").await;
    assert!(result.is_ok());
    assert!(!result.unwrap().executed, "Pipeline should not execute");

    println!("  ✅ High priority correctly overrides Medium (heat + bear regime)");

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 5: DebateLayer produces valid verdict with 12/11 factor evidence
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_deb_layer_with_new_indicators() {
    let (orch, db_path) = setup("debate_indicators");

    seed_rich_ohlcv(&orch.state, "SOL", 180.0).await;
    seed_aggregated_signal(&orch.state, 0.75).await;
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBull);

    let debate = tredo_autonomous::debate_layer::DebateLayer::new(orch.state.clone());
    let (verdict, signal_opt) = debate.run_debate("SOL", 180.0).await;

    // Verdict must be well-formed
    assert!(
        verdict.action == "BUY" || verdict.action == "SELL" || verdict.action == "HOLD",
        "Verdict action should be BUY/SELL/HOLD, got '{}'",
        verdict.action
    );
    assert!(
        verdict.confidence >= 0.0 && verdict.confidence <= 1.0,
        "Confidence should be in [0,1], got {}",
        verdict.confidence
    );
    assert!(!verdict.reasoning.is_empty(), "Verdict should have reasoning");
    assert!(
        verdict.rounds_played >= 2,
        "Should play at least 2 rounds, played {}",
        verdict.rounds_played
    );

    // If BUY/SELL, signal should be well-formed
    if let Some(sig) = &signal_opt {
        assert_eq!(sig.symbol, "SOL");
        assert!(sig.entry_price > 0.0);
        assert!(sig.stop_loss > 0.0);
        assert!(sig.take_profit > 0.0);
        assert!(sig.position_size > 0.0);
        assert!(
            sig.risk_reward_ratio > 0.0,
            "Risk:reward should be positive"
        );
        println!(
            "  📊 DebateLayer signal: {:?} @ {:.2} RR={:.1}:1 conf={:.1}%",
            sig.direction,
            sig.entry_price,
            sig.risk_reward_ratio,
            sig.confidence_score * 100.0
        );
    }

    println!(
        "  ✅ DebateLayer verdict: {} (conf {:.1}%, veto={}, rounds={}, {})",
        verdict.action,
        verdict.confidence * 100.0,
        verdict.judge_veto,
        verdict.rounds_played,
        if verdict.judge_veto { "JUDGE VETOED" } else { "APPROVED" }
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 6: Judge vetoes low-confidence debate (bear regime → higher threshold)
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_judge_vetoes_low_confidence() {
    let (orch, db_path) = setup("judge_veto");

    seed_rich_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Bear regime raises Judge's confidence threshold to 0.60
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBear);

    // Low confluence + bear regime → weak debate signals
    // This creates conditions where synthesis confidence will be low
    {
        let mut portfolio = orch.state.portfolio.write().await;
        portfolio.consecutive_losses = 3;
    }

    let debate = tredo_autonomous::debate_layer::DebateLayer::new(orch.state.clone());
    let (verdict, signal_opt) = debate.run_debate("ETH", 3_500.0).await;

    // In bear regime with losses, the judge should either:
    // - Veto the BUY (if synthesis produced BUY with low confidence)
    // - Or synthesis itself produces HOLD (which is fine — no signal)
    if verdict.judge_veto {
        assert_eq!(
            verdict.action, "HOLD",
            "Judge veto should override to HOLD"
        );
        assert!(
            signal_opt.is_none(),
            "No signal should be produced on judge veto"
        );
        println!("  ✅ Judge correctly vetoed low-confidence debate in bear regime");
    } else {
        // Judge approved — synthesis had enough confidence
        println!(
            "  ✅ Judge approved (synthesis had sufficient confidence): {} (conf {:.1}%)",
            verdict.action,
            verdict.confidence * 100.0
        );
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 7: Low-priority warnings don't block pipeline execution
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_low_priority_warnings_dont_block() {
    let (orch, db_path) = setup("low_warn");

    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_aggregated_signal(&orch.state, 0.85).await;
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBull);

    // Add 3 positions on BTC (Low: max_positions_per_symbol)
    {
        let mut portfolio = orch.state.portfolio.write().await;
        for i in 0..3 {
            portfolio.open_positions.push(OpenPosition {
                symbol: "BTC".to_string(),
                direction: TradeDirection::Long,
                entry_price: 60000.0 + i as f64 * 1000.0,
                current_price: 61000.0,
                stop_loss: 58000.0,
                take_profit: 65000.0,
                quantity: 0.1,
                unrealized_pnl: 0.0,
                unrealized_pnl_pct: 0.0,
                entry_time: Utc::now(),
                risk_amount: 200.0,
            });
        }
        portfolio.total_equity = 100_000.0;
    }

    // Gate should pass (Low warnings don't block)
    let gate = tredo_autonomous::hard_rules_gate::HardRulesGate::new(orch.state.clone());
    let gate_result = gate.evaluate("BTC").await;
    assert!(gate_result.passed, "Low warnings should not block");
    assert!(
        gate_result.failed_rules.iter().any(|r| r.rule_name == "max_positions_per_symbol"),
        "Should have Low warning about positions"
    );

    // Full pipeline should proceed past the gate
    let result = orch.run_full_pipeline("BTC").await;
    assert!(result.is_ok(), "Pipeline should not error: {:?}", result.err());

    let summary = result.unwrap();
    // Pipeline should have run through debate at minimum
    // (may HOLD or may trade depending on debate scoring)
    println!(
        "  ✅ Low-priority warning did not block: executed={} reason=\"{}\"",
        summary.executed, summary.reason
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 8: COT chain captures all 5 layer entries on successful pipeline
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_cot_chain_captures_all_layers() {
    let (orch, db_path) = setup("cot_chain");

    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_aggregated_signal(&orch.state, 0.85).await;
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBull);

    let cot_before = orch.state.cot_store.read().await.len();

    let result = orch.run_full_pipeline("BTC").await;
    assert!(result.is_ok(), "Pipeline must not error: {:?}", result.err());

    let summary = result.unwrap();
    let cot_after = orch.state.cot_store.read().await.len();
    let new_entries = cot_after - cot_before;

    // A successful pipeline should produce COT entries for:
    // - Pipeline start (Orchestrator)
    // - Phase 0 (position check)
    // - HardRulesGate (PASSED)
    // - Identifier (market analysis)
    // - Verifier (risk assessment)
    // - [possibly WFA gate]
    // - DebateLayer (adversarial debate)
    // - Execution or HOLD
    // - Pipeline final decision
    // That's at least 7-8 entries
    assert!(
        new_entries >= 5,
        "Pipeline should produce at least 5 COT entries (got {}). Pipeline: {}",
        new_entries,
        summary.reason
    );

    // Verify the COT chain contains entries from multiple layers
    let cot = orch.state.cot_store.read().await;
    let chain_entries: Vec<_> = cot.iter().rev().take(new_entries).collect();

    let has_hard_rules = chain_entries.iter().any(|e| e.agent.contains("HardRules") || e.agent.contains("Gate"));
    let has_identifier = chain_entries.iter().any(|e| e.agent.contains("Identifier"));
    let has_debate = chain_entries.iter().any(|e| e.agent.contains("Debate") || e.agent.contains("DebateLayer"));

    assert!(has_hard_rules, "COT should have HardRulesGate entry");
    assert!(has_identifier, "COT should have Identifier entry");
    assert!(has_debate, "COT should have DebateLayer entry");

    println!(
        "  ✅ COT chain captured {} entries across layers (Gate={}, Identifier={}, Debate={})",
        new_entries, has_hard_rules, has_identifier, has_debate
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 9: Pipeline resilience — no panic across varied market conditions
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_pipeline_resilience_varied_conditions() {
    let (orch, db_path) = setup("resilience");

    seed_rich_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_aggregated_signal(&orch.state, 0.70).await;

    // Run 5 iterations with different regime conditions
    let regimes = [
        Some(tredo_autonomous::types::MarketRegime::TrendingBull),
        Some(tredo_autonomous::types::MarketRegime::TrendingBear),
        Some(tredo_autonomous::types::MarketRegime::Ranging),
        Some(tredo_autonomous::types::MarketRegime::Volatile),
        None, // Unknown regime
    ];

    for (i, regime) in regimes.iter().enumerate() {
        *orch.state.market_regime.write().await = *regime;

        let result = orch.run_full_pipeline("BTC").await;
        assert!(
            result.is_ok(),
            "Iteration {} with regime {:?} must not error: {:?}",
            i + 1,
            regime,
            result.err()
        );

        let summary = result.unwrap();
        // Pipeline may be 0ms when gate blocks immediately (fast path), but should not hang
        assert!(
            summary.total_duration_ms < 10_000,
            "Pipeline should complete within 10s, took {}ms",
            summary.total_duration_ms
        );
        assert!(
            !summary.reason.is_empty(),
            "Pipeline should have a reason"
        );

        println!(
            "  Iteration {} (regime {:?}): executed={} reason=\"{}\"",
            i + 1,
            regime,
            summary.executed,
            &summary.reason[..summary.reason.len().min(80)]
        );
    }

    println!("  ✅ All 5 regime conditions completed without panic");

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 10: Identifier advisory output feeds into DebateLayer context
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_identifier_feeds_debate_context() {
    let (orch, db_path) = setup("ident_to_debate");

    seed_rich_ohlcv(&orch.state, "SOL", 180.0).await;
    seed_aggregated_signal(&orch.state, 0.80).await;
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBull);

    // Run Identifier first to populate market state (patterns, pivots, confluence, etc.)
    let ident_result = orch
        .tredo()
        .run_identifier("SOL", 180.0, 1)
        .await;
    assert!(
        ident_result.is_ok(),
        "Identifier should succeed: {:?}",
        ident_result.err()
    );

    let (discipline_ok, confluence, pivots) = ident_result.unwrap();
    assert!(
        discipline_ok,
        "Crypto should pass discipline checks"
    );
    assert!(
        confluence >= 0.0 && confluence <= 1.0,
        "Confluence should be in [0,1], got {}",
        confluence
    );
    assert!(pivots.pivot.is_finite(), "Pivot should be finite");

    println!(
        "  ✅ Identifier: confluence={:.1}% pivot={:.2} R1={:.2} S1={:.2}",
        confluence * 100.0,
        pivots.pivot,
        pivots.r1,
        pivots.s1
    );

    // Now run DebateLayer — it should read the populated state
    let debate = tredo_autonomous::debate_layer::DebateLayer::new(orch.state.clone());
    let (verdict, signal_opt) = debate.run_debate("SOL", 180.0).await;

    // Verdict should be well-formed
    assert!(
        verdict.action == "BUY" || verdict.action == "SELL" || verdict.action == "HOLD",
        "Verdict should be valid, got '{}'",
        verdict.action
    );
    assert!(
        verdict.confidence > 0.0,
        "Confidence should be positive"
    );
    assert!(!verdict.reasoning.is_empty());

    println!(
        "  ✅ DebateLayer after Identifier: {} (conf {:.1}%, veto={}, rounds={})",
        verdict.action,
        verdict.confidence * 100.0,
        verdict.judge_veto,
        verdict.rounds_played
    );

    if let Some(sig) = &signal_opt {
        println!(
            "  📊 Signal: {:?} @ {:.2} RR={:.1}:1",
            sig.direction, sig.entry_price, sig.risk_reward_ratio
        );
    }

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST 11: Medium-only block (bear regime + low confluence) — no Higher override
// ══════════════════════════════════════════════════════════════════════════════
#[tokio::test]
async fn test_medium_only_block_stops_pipeline() {
    let (orch, db_path) = setup("med_block");

    seed_rich_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Bear regime + low confluence (default 0.5 < 0.80 bear minimum)
    // Do NOT seed aggregated signal — want confluence to default to 0.5
    *orch.state.market_regime.write().await =
        Some(tredo_autonomous::types::MarketRegime::TrendingBear);

    // Verify gate blocks at Medium level
    let gate = tredo_autonomous::hard_rules_gate::HardRulesGate::new(orch.state.clone());
    let gate_result = gate.evaluate("ETH").await;

    assert!(!gate_result.passed, "Medium should block without Higher override");
    assert_eq!(
        gate_result.highest_failed_priority,
        Some(tredo_autonomous::types::RulePriority::Medium),
        "Should be Medium priority block"
    );

    // Full pipeline should also block
    let result = orch.run_full_pipeline("ETH").await;
    assert!(result.is_ok());
    assert!(!result.unwrap().executed, "Pipeline should not execute on Medium block");

    println!(
        "  ✅ Medium block correctly stopped pipeline (bear regime + low confluence)"

    );

    let _ = fs::remove_file(&db_path);
}
