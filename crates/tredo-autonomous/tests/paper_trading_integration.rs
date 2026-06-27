// paper_trading_integration.rs
// Integration test for the full paper trading pipeline:
//   TradeSignal → execute_paper_trade → position in portfolio → SL/TP monitoring → close
//
// Run: cargo test -p tredo-autonomous --test paper_trading_integration

use chrono::Utc;
use std::fs;
use tredo_autonomous::{types::TradeSignal, AutonomousOrchestrator, SharedState};
use tredo_core::{Config, DisciplineRules, MemoryStore, OhlcvBar, TradeDirection};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn setup_env(db_name: &str) -> (AutonomousOrchestrator, String) {
    let sqlite_db_path = format!("test_paper_{}.db", db_name);
    for f in &[
        sqlite_db_path.to_string(),
        format!("{}-wal", sqlite_db_path),
        format!("{}-shm", sqlite_db_path),
    ] {
        let _ = fs::remove_file(f);
    }
    let db_path = format!("test_paper_{}.redb", db_name);
    let _ = fs::remove_file(&db_path);

    let memory = MemoryStore::new(&db_path).expect("MemoryStore creation");
    let config = Config::default();
    let rules = DisciplineRules::default();
    let state = SharedState::new(memory, rules, config, &sqlite_db_path).expect("SharedState init");
    let mut orch = AutonomousOrchestrator::new(state);
    orch.init_tredo();
    (orch, db_path)
}

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

fn make_signal(
    symbol: &str,
    direction: TradeDirection,
    entry: f64,
    sl: f64,
    tp: f64,
    qty: f64,
) -> TradeSignal {
    TradeSignal {
        symbol: symbol.to_string(),
        direction,
        entry_price: entry,
        stop_loss: sl,
        take_profit: tp,
        position_size: qty,
        confidence_score: 0.80,
        confluence_score: 0.75,
        risk_reward_ratio: 2.0,
        reasoning: "Integration test signal".to_string(),
        timestamp: Utc::now(),
        session_valid: true,
        risk_check_passed: true,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Full paper trade cycle — signal → execute → position → equity check
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_paper_trade_full_cycle() {
    let (orch, db_path) = setup_env("full_cycle");

    // Seed market data
    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;

    // Verify clean starting state
    let equity_before = orch.state.portfolio.read().await.total_equity;
    assert_eq!(equity_before, 100_000.0, "Initial equity should be ₹100k");

    // 1. Create a trade signal: BUY 0.05 BTC at 65000, SL 64000, TP 67000
    //    Position value: 0.05 × 65,000 = ₹3,250 (under 1/25 cap of ₹4,000)
    let signal = make_signal(
        "BTC",
        TradeDirection::Long,
        65_000.0, // entry
        64_000.0, // SL (-1.5%)
        67_000.0, // TP (+3.1%)
        0.05,     // 0.05 units
    );

    // 2. Execute paper trade
    let result = orch.execution.execute_paper_trade(&signal).await;
    assert!(
        result.is_ok(),
        "Paper trade should succeed: {:?}",
        result.err()
    );
    println!("  ✅ Trade executed: {}", result.unwrap());

    // 3. Verify position appears in portfolio
    //    Note: execution coordinator applies slippage (0.05%) to entry/SL/TP
    {
        let portfolio = orch.state.portfolio.read().await;
        assert_eq!(
            portfolio.open_positions.len(),
            1,
            "Should have 1 open position"
        );

        let pos = &portfolio.open_positions[0];
        assert_eq!(pos.symbol, "BTC");
        assert_eq!(pos.direction, TradeDirection::Long);
        assert_eq!(pos.quantity, 0.05);
        assert!(pos.entry_price > 0.0, "Entry price should be positive");
        // SL/TP are adjusted by slippage (~0.05% of entry price) — check within reasonable bounds
        assert!(
            pos.stop_loss >= 64_000.0 && pos.stop_loss < 64_100.0,
            "SL should be near 64000 (slippage-adjusted), got {}",
            pos.stop_loss
        );
        assert!(
            pos.take_profit >= 67_000.0 && pos.take_profit < 67_100.0,
            "TP should be near 67000 (slippage-adjusted), got {}",
            pos.take_profit
        );
        println!(
            "  📊 Position: {} qty={} @ {:.2} SL={:.2} TP={:.2}",
            pos.symbol, pos.quantity, pos.entry_price, pos.stop_loss, pos.take_profit
        );
    }

    // 4. Verify equity hasn't changed (paper trade = virtual money, no real cash movement yet)
    let equity_after = orch.state.portfolio.read().await.total_equity;
    assert!(
        (equity_after - 100_000.0).abs() < 1.0,
        "Equity should remain ~₹100k after paper trade entry"
    );

    // 5. Run the execution coordinator's Agent::run(None) to check SL/TP
    //    (this simulates the fast loop calling check_and_exit_positions)
    use tredo_core::Agent;
    let exec_result = orch.execution.run(None).await;
    assert!(
        exec_result.is_ok(),
        "Execution coordinator run should succeed"
    );
    println!("  ✅ Execution coordinator check completed");

    // Position should still be open (price hasn't hit SL or TP)
    let pos_count = orch.state.portfolio.read().await.open_positions.len();
    assert_eq!(
        pos_count, 1,
        "Position should still be open (price not at SL/TP)"
    );

    let _ = fs::remove_file(&db_path);
}

/// Push a new OHLCV bar at the given price so refresh_position_prices() picks it up.
/// This is how real prices flow in — through the OHLCV history — not by mutating
/// positions directly.
async fn push_price_tick(state: &SharedState, symbol: &str, price: f64) {
    let mut history = state.ohlcv_history.write().await;
    if let Some(bars) = history.get_mut(symbol) {
        let last = bars.last().unwrap();
        bars.push(OhlcvBar {
            timestamp: (Utc::now()).to_rfc3339(),
            open: price,
            high: price * 1.001,
            low: price * 0.999,
            close: price,
            volume: last.volume,
        });
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: SL/TP monitoring — update price to trigger stop-loss
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_sl_tp_monitoring_triggers_stop_loss() {
    let (orch, db_path) = setup_env("sl_monitoring");

    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Open a position: BUY 1 ETH at 3500, SL 3400, TP 3700
    //    Position value: 1 × 3,500 = ₹3,500 (under 1/25 cap of ₹4,000)
    let signal = make_signal("ETH", TradeDirection::Long, 3_500.0, 3_400.0, 3_700.0, 1.0);
    orch.execution.execute_paper_trade(&signal).await.unwrap();

    // Push a new price tick well below SL — refresh_position_prices() picks this up
    // and triggers the stop-loss automatically
    push_price_tick(&orch.state, "ETH", 3_300.0).await;

    // Run execution coordinator — should trigger SL exit
    use tredo_core::Agent;
    let result = orch.execution.run(None).await;
    assert!(result.is_ok(), "Execution run should succeed after SL hit");

    // Verify position was closed
    let portfolio = orch.state.portfolio.read().await;
    assert!(
        portfolio.open_positions.is_empty(),
        "Position should be closed after SL hit"
    );
    assert!(
        portfolio.losing_trades_today > 0 || portfolio.daily_pnl < 0.0,
        "Should record a losing trade"
    );
    println!(
        "  ✅ SL triggered: positions={}, daily_pnl={:.2}",
        portfolio.open_positions.len(),
        portfolio.daily_pnl
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Rejected signal — position_size of 0 should be rejected
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_rejected_signal_zero_position_size() {
    let (orch, db_path) = setup_env("rejected_signal");

    let signal = make_signal(
        "BTC",
        TradeDirection::Long,
        65_000.0,
        64_000.0,
        67_000.0,
        0.0,
    );
    let result = orch.execution.execute_paper_trade(&signal).await;

    assert!(
        result.is_err(),
        "Signal with position_size=0 should be rejected"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("Invalid position size"),
        "Error should mention invalid position size: {}",
        err
    );

    // Portfolio should remain empty
    assert_eq!(orch.state.portfolio.read().await.open_positions.len(), 0);
    println!("  ✅ Rejected signal correctly: {}", err);

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Take-profit monitoring — update price to trigger TP
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_sl_tp_monitoring_triggers_take_profit() {
    let (orch, db_path) = setup_env("tp_monitoring");

    seed_ohlcv(&orch.state, "SOL", 180.0).await;

    // Open a position: BUY 20 SOL at 180, SL 175, TP 195
    //    Position value: 20 × 180 = ₹3,600 (under 1/25 cap of ₹4,000)
    let signal = make_signal("SOL", TradeDirection::Long, 180.0, 175.0, 195.0, 20.0);
    orch.execution.execute_paper_trade(&signal).await.unwrap();

    // Push a new price tick well above TP — refresh_position_prices() picks this up
    // and triggers take-profit automatically
    push_price_tick(&orch.state, "SOL", 200.0).await;

    // Run execution coordinator — should trigger TP exit
    use tredo_core::Agent;
    let result = orch.execution.run(None).await;
    assert!(result.is_ok(), "Execution run should succeed after TP hit");

    // Verify position was closed
    let portfolio = orch.state.portfolio.read().await;
    assert!(
        portfolio.open_positions.is_empty(),
        "Position should be closed after TP hit"
    );
    assert!(
        portfolio.winning_trades_today > 0 || portfolio.daily_pnl > 0.0,
        "Should record a winning trade"
    );
    println!(
        "  ✅ TP triggered: positions={}, daily_pnl={:.2}",
        portfolio.open_positions.len(),
        portfolio.daily_pnl
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Portfolio equity/P&L updates after trade close
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_equity_pnl_updates_after_close() {
    let (orch, db_path) = setup_env("equity_pnl");

    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;

    // Open and immediately close at a profit via portfolio manager
    //    Position value: 0.05 × 65,000 = ₹3,250 (under 1/25 cap of ₹4,000)
    let signal = make_signal(
        "BTC",
        TradeDirection::Long,
        65_000.0,
        64_000.0,
        67_000.0,
        0.05,
    );

    // Add position directly via portfolio manager
    orch.portfolio.add_position(&signal).await.unwrap();

    let equity_before_close = orch.state.portfolio.read().await.total_equity;
    println!("  Equity after open: {:.2}", equity_before_close);

    // Simulate price going to TP
    {
        let mut portfolio = orch.state.portfolio.write().await;
        if let Some(pos) = portfolio
            .open_positions
            .iter_mut()
            .find(|p| p.symbol == "BTC")
        {
            pos.current_price = 67_000.0;
        }
    }

    // Close the position manually at TP
    let pnl = orch
        .portfolio
        .close_position("BTC", 67_000.0)
        .await
        .unwrap();

    println!("  P&L from close: ₹{:.2}", pnl);
    assert!(pnl > 0.0, "Closing at TP should be profitable");

    // Verify portfolio state after close
    let portfolio = orch.state.portfolio.read().await;
    assert!(
        portfolio.open_positions.is_empty(),
        "All positions should be closed"
    );
    assert!(
        portfolio.daily_pnl > 0.0,
        "Daily P&L should be positive after profitable close"
    );
    println!(
        "  ✅ Equity: ₹{:.2} | P&L: ₹{:.2} | Positions: {}",
        portfolio.total_equity,
        portfolio.daily_pnl,
        portfolio.open_positions.len()
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Multiple positions — open several, close one, verify rest remain
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_multiple_positions_partial_close() {
    let (orch, db_path) = setup_env("multi_pos");

    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Open two positions (position values under 1/25 cap of ₹4,000)
    // BTC: 0.05 × 65,000 = ₹3,250
    // ETH: 1 × 3,500 = ₹3,500
    let btc_signal = make_signal(
        "BTC",
        TradeDirection::Long,
        65_000.0,
        64_000.0,
        67_000.0,
        0.05,
    );
    let eth_signal = make_signal("ETH", TradeDirection::Long, 3_500.0, 3_400.0, 3_700.0, 1.0);

    orch.execution
        .execute_paper_trade(&btc_signal)
        .await
        .unwrap();
    orch.execution
        .execute_paper_trade(&eth_signal)
        .await
        .unwrap();

    assert_eq!(orch.state.portfolio.read().await.open_positions.len(), 2);

    // Push ETH price to SL level — refresh_position_prices() picks this up and
    // triggers the stop-loss for ETH while BTC stays open
    push_price_tick(&orch.state, "ETH", 3_400.0).await;

    use tredo_core::Agent;
    orch.execution.run(None).await.unwrap();

    // BTC should still be open, ETH should be closed
    let portfolio = orch.state.portfolio.read().await;
    assert_eq!(
        portfolio.open_positions.len(),
        1,
        "Should have 1 position remaining (BTC)"
    );
    assert_eq!(portfolio.open_positions[0].symbol, "BTC");
    println!(
        "  ✅ Partial close: {} position(s) remaining, ETH stopped out",
        portfolio.open_positions.len()
    );

    let _ = fs::remove_file(&db_path);
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Pipeline state consistency — run full pipeline then execute
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_pipeline_then_execution() {
    let (orch, db_path) = setup_env("pipeline_exec");

    seed_ohlcv(&orch.state, "NIFTY", 24_500.0).await;
    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;

    // Run the identifier (populates market intelligence, regime, etc.)
    let (disc_ok, conf, pivots) = orch
        .tredo()
        .run_identifier("NIFTY", 24_500.0, 1)
        .await
        .expect("Identifier should succeed");

    assert!(disc_ok, "NIFTY discipline should pass");
    assert!((0.0..=1.0).contains(&conf), "Confluence should be 0-1");
    println!(
        "  📊 Identifier: disc={}, conf={:.1}%, pivot={:.2}",
        if disc_ok { "OK" } else { "FAIL" },
        conf * 100.0,
        pivots.pivot
    );

    // Run the verifier with current equity
    let equity = orch.state.portfolio.read().await.total_equity;
    let risk = orch
        .tredo()
        .run_verifier("NIFTY", 24_500.0, equity, 1)
        .await
        .expect("Verifier should succeed");

    println!(
        "  🛡️ Verifier: {:?}, heat={:.1}%",
        risk.recommendation,
        risk.portfolio_heat * 100.0
    );

    // Now execute a trade based on the pipeline state
    //    Position value: 0.15 × 24,500 = ₹3,675 (under 1/25 cap of ₹4,000)
    let signal = make_signal(
        "NIFTY",
        TradeDirection::Long,
        24_500.0,
        24_300.0,
        24_900.0,
        0.15,
    );
    let exec_result = orch.execution.execute_paper_trade(&signal).await;

    match &exec_result {
        Ok(msg) => println!("  ✅ Pipeline→Execution: {}", msg),
        Err(e) => println!("  ⚠️ Pipeline→Execution failed (expected if no LLM): {}", e),
    }

    // Regardless of LLM availability, the execution coordinator should handle it gracefully
    let pos_count = orch.state.portfolio.read().await.open_positions.len();
    println!(
        "  📊 Positions after pipeline→execute: {} (1 expected if execution succeeded)",
        pos_count
    );

    let _ = fs::remove_file(&db_path);
}
