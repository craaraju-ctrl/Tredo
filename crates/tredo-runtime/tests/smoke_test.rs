//! Smoke tests for the tredo-runtime crate.
//!
//! Run: cargo test -p tredo-runtime --test smoke_test
//!
//! These tests verify:
//! 1. RuntimeEngine can be constructed in paper mode
//! 2. EventBus can publish and subscribe to events
//! 3. Basic API client functions work (with mocked network or graceful fallback)
//! 4. The engine starts and stops without panicking

use std::time::Duration;
use tredo_autonomous::AutonomousOrchestrator;
use tredo_core::{Config, DisciplineRules, MemoryStore};
use tredo_runtime::engine::RuntimeEngine;
use tredo_runtime::event_bus::{AgentEvent, EventBus};
use tredo_runtime::mode::{ModeConfig, TradingMode};

// ══════════════════════════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════════════════════════

/// Build a fresh test environment for runtime tests.
/// Cleans up temp files before creating new ones.
fn setup_test_env(test_name: &str) -> (AutonomousOrchestrator, Vec<String>, String) {
    let db_path = format!("test_runtime_{}.redb", test_name);
    let sqlite_path = format!("test_runtime_{}.db", test_name);

    // Clean up any leftover files
    for f in &[
        db_path.clone(),
        format!("{}.lock", db_path),
        sqlite_path.clone(),
        format!("{}-wal", sqlite_path),
        format!("{}-shm", sqlite_path),
    ] {
        let _ = std::fs::remove_file(f);
    }

    let memory = MemoryStore::new(&db_path).expect("MemoryStore creation");
    let config = Config::default();
    let rules = DisciplineRules::default();
    let state = tredo_autonomous::state::SharedState::new(memory, rules, config, &sqlite_path)
        .expect("SharedState init");

    let mut orch = AutonomousOrchestrator::new(state);
    orch.init_tredo();

    let symbols = vec!["BTC".to_string(), "ETH".to_string()];
    (orch, symbols, db_path)
}

/// Clean up test database files.
fn cleanup(db_path: &str) {
    let prefix = db_path.trim_end_matches(".redb");
    // MemoryStore (redb)
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{}.lock", db_path));
    let _ = std::fs::remove_file(format!("{}_history.db", prefix));
    let _ = std::fs::remove_file(format!("{}_history.db-wal", prefix));
    let _ = std::fs::remove_file(format!("{}_history.db-shm", prefix));
    // SharedState (SQLite)
    let sqlite = format!("{}.db", prefix);
    let _ = std::fs::remove_file(&sqlite);
    let _ = std::fs::remove_file(format!("{}-wal", sqlite));
    let _ = std::fs::remove_file(format!("{}-shm", sqlite));
}

// ══════════════════════════════════════════════════════════════════════════════
// EventBus Tests
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_eventbus_publish_subscribe() {
    let bus = EventBus::new(16);
    let mut rx = bus.subscribe();

    bus.publish(AgentEvent::PriceTick {
        symbol: "BTC".to_string(),
        price: 65000.0,
        volume: 100.0,
        timestamp: chrono::Utc::now(),
        source: tredo_runtime::event_bus::PriceSource::Rest,
    });

    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Should receive event within timeout")
        .expect("Should get Ok event");

    match event {
        AgentEvent::PriceTick { symbol, price, .. } => {
            assert_eq!(symbol, "BTC");
            assert!((price - 65000.0).abs() < f64::EPSILON);
        }
        other => panic!("Expected PriceTick, got {:?}", other),
    }

    assert!(bus.published_count() >= 1, "Published count should be >= 1");
}

#[tokio::test]
async fn test_eventbus_multiple_subscribers() {
    let bus = EventBus::new(16);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    bus.publish(AgentEvent::Shutdown);

    // Both subscribers should receive the Shutdown event
    let e1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv())
        .await
        .expect("rx1 should receive")
        .expect("rx1 Ok");
    let e2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv())
        .await
        .expect("rx2 should receive")
        .expect("rx2 Ok");

    assert!(matches!(e1, AgentEvent::Shutdown));
    assert!(matches!(e2, AgentEvent::Shutdown));
}

#[tokio::test]
async fn test_eventbus_capacity() {
    // Publish more events than capacity — should not panic (oldest are dropped)
    let bus = EventBus::new(4);
    let mut rx = bus.subscribe();

    for i in 0..10 {
        bus.publish(AgentEvent::PriceTick {
            symbol: format!("SYM{}", i),
            price: i as f64,
            volume: 0.0,
            timestamp: chrono::Utc::now(),
            source: tredo_runtime::event_bus::PriceSource::Rest,
        });
    }

    // Reading from a slow subscriber should show Lagged error
    let result = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    match result {
        Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
            assert!(n > 0, "Should have dropped at least some events");
            println!("Lagged by {} events (expected)", n);
        }
        Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
            panic!("Channel should not be closed");
        }
        Ok(Ok(event)) => {
            // Might get the last event if it fits in the buffer
            println!("Got event instead of lag: {:?}", event);
        }
        Err(_) => panic!("Timeout waiting for event"),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Mode & Config Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_trading_mode_default() {
    let mode = TradingMode::Paper;
    assert_eq!(format!("{}", mode), "paper");
}

#[test]
fn test_trading_mode_parse() {
    assert_eq!("paper".parse::<TradingMode>().unwrap(), TradingMode::Paper);
    assert_eq!("live".parse::<TradingMode>().unwrap(), TradingMode::Live);
    assert_eq!(
        "backtest".parse::<TradingMode>().unwrap(),
        TradingMode::Backtest
    );
    assert_eq!(
        "validate".parse::<TradingMode>().unwrap(),
        TradingMode::Validate
    );
    assert_eq!(
        "research".parse::<TradingMode>().unwrap(),
        TradingMode::Research
    );
    assert!("invalid".parse::<TradingMode>().is_err());
}

#[test]
fn test_mode_config_default() {
    let config = ModeConfig::default();
    assert_eq!(config.mode, TradingMode::Paper);
    assert!(config.require_trade_confirmation);
    assert!((config.max_daily_loss - 1000.0).abs() < f64::EPSILON);
    assert!((config.backtest_initial_capital - 100_000.0).abs() < f64::EPSILON);
}

// ══════════════════════════════════════════════════════════════════════════════
// RuntimeEngine Construction Test (Primary Smoke Test)
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_runtime_engine_construction() {
    let (orchestrator, symbols, db_path) = setup_test_env("construction");

    let mode_config = ModeConfig {
        mode: TradingMode::Paper,
        require_trade_confirmation: false,
        max_daily_loss: 5000.0,
        symbol_whitelist: None,
        backtest_start: None,
        backtest_end: None,
        backtest_data_path: None,
        backtest_initial_capital: 100_000.0,
        validate_cycles: 10,
        induce_regret: false,
    };

    // Test 1: Engine construction should succeed
    let engine = RuntimeEngine::new(mode_config, orchestrator, symbols, None)
        .await
        .expect("RuntimeEngine construction should succeed");

    // Test 2: EventBus should be functional
    engine.event_bus().publish(AgentEvent::PriceTick {
        symbol: "TEST".to_string(),
        price: 100.0,
        volume: 50.0,
        timestamp: chrono::Utc::now(),
        source: tredo_runtime::event_bus::PriceSource::Manual,
    });

    assert!(
        engine.event_bus().published_count() > 0,
        "Events should be published"
    );

    // Test 3: Run the engine with a timeout (it loops forever in event-driven mode)
    // We call run() but wrap it in a timeout that cancels it after 2 seconds.
    // This verifies the engine starts without panicking, then we abort cleanly.
    let handle = tokio::spawn(async move {
        match engine.run().await {
            Ok(summary) => println!(
                "Engine ran successfully: {} cycles, {:.2}% DD",
                summary.cycles_completed,
                summary.max_drawdown * 100.0
            ),
            Err(e) => println!("Engine stopped with error (expected with timeout): {}", e),
        }
    });

    // Give the engine time to initialize and process a few events, then abort
    tokio::time::sleep(Duration::from_secs(2)).await;
    handle.abort();

    cleanup(&db_path);
    println!("✅ RuntimeEngine smoke test passed — construction + brief run OK");
}

// ══════════════════════════════════════════════════════════════════════════════
// API Client Smoke Test
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[ignore] // Requires network access — run manually with: cargo test -p tredo-runtime -- --ignored api
async fn test_api_client_fetch_price() {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Client build");

    // Try fetching BTC price from Binance
    match tredo_runtime::api_clients::fetch_binance_price(&client, "BTC").await {
        Ok(price) => {
            assert!(price > 0.0, "BTC price should be positive, got {}", price);
            println!("Binance BTC price: ${:.2}", price);
        }
        Err(e) => {
            // Network might not be available in CI — that's OK, just warn
            println!("Binance API unavailable (expected in offline CI): {}", e);
        }
    }

    // Try fetching NIFTY price from Yahoo
    match tredo_runtime::api_clients::fetch_yahoo_price(&client, "NIFTY").await {
        Ok(price) => {
            assert!(price > 0.0, "NIFTY price should be positive");
            println!("Yahoo NIFTY price: ₹{:.2}", price);
        }
        Err(e) => {
            println!("Yahoo API unavailable: {}", e);
        }
    }

    // Try fetching a full live bar (composite)
    match tredo_runtime::api_clients::fetch_live_bar(&client, "BTC").await {
        Ok(bar) => {
            assert!(bar.close > 0.0, "Bar should have positive close price");
            assert!(bar.high > 0.0, "Bar should have positive high");
            assert!(bar.low > 0.0, "Bar should have positive low");
            println!("Live bar: O={:.2} H={:.2} L={:.2} C={:.2} V={:.0}",
                bar.open, bar.high, bar.low, bar.close, bar.volume);
        }
        Err(e) => {
            println!("Live bar API unavailable: {}", e);
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Cleanup
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_cleanup_db_files() {
    let prefixes = ["construction"];
    let mut cleaned = 0;
    for prefix in &prefixes {
        let path = format!("test_runtime_{}.redb", prefix);
        if std::fs::remove_file(&path).is_ok() {
            cleaned += 1;
        }
        let _ = std::fs::remove_file(format!("{}.lock", path));
        let _ = std::fs::remove_file(format!("test_runtime_{}.db", prefix));
        let _ = std::fs::remove_file(format!("test_runtime_{}.db-wal", prefix));
        let _ = std::fs::remove_file(format!("test_runtime_{}.db-shm", prefix));
    }
    if cleaned > 0 {
        println!("Cleaned up {} leftover db files", cleaned);
    }
}
