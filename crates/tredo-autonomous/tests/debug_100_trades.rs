// debug_100_trades.rs
// Debug script: runs 100 simulated trades and logs which blocking point each trade hits.
// Each trade flows through the full 5-layer pipeline (Gate → Identifier → Verifier → Debate → Execution).
// The output shows where each trade stops and a summary of blocking point distribution.
//
// Run: cargo test -p tredo-autonomous --test debug_100_trades -- --nocapture

use chrono::Utc;
use std::collections::HashMap;
use std::fs;
use tredo_autonomous::types::{MarketRegime, OpenPosition};
use tredo_autonomous::{AutonomousOrchestrator, SharedState};
use tredo_core::{Config, DisciplineRules, MemoryStore, OhlcvBar, TradeDirection};

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn setup_env(db_name: &str) -> (AutonomousOrchestrator, String) {
    let sqlite_db_path = format!("test_debug_{}.db", db_name);
    for f in &[
        sqlite_db_path.to_string(),
        format!("{}-wal", sqlite_db_path),
        format!("{}-shm", sqlite_db_path),
    ] {
        let _ = fs::remove_file(f);
    }
    let db_path = format!("test_debug_{}.redb", db_name);
    let _ = fs::remove_file(&db_path);

    let memory = MemoryStore::new(&db_path).expect("MemoryStore creation");
    let config = Config {
        kronos_service_url: "http://127.0.0.1:19999".to_string(),
        ..Config::default()
    };
    let rules = DisciplineRules::default();
    let state = SharedState::new(memory, rules, config, &sqlite_db_path).expect("SharedState init");
    // Clear calendar events so red_folder Critical rule doesn't fire
    *state.calendar_events.write().await = Vec::new();

    let mut orch = AutonomousOrchestrator::new(state);
    orch.init_tredo();
    (orch, db_path)
}

async fn seed_ohlcv(state: &SharedState, symbol: &str, base_price: f64) {
    let mut history = state.ohlcv_history.write().await;
    let mut bars = Vec::with_capacity(120);
    for i in 0..120 {
        let trend = base_price * (i as f64 * 0.002);
        let noise = (i as f64 * 0.5).sin() * base_price * 0.01;
        let close = base_price + trend + noise;
        bars.push(OhlcvBar {
            timestamp: (Utc::now() - chrono::Duration::minutes(120 - i as i64)).to_rfc3339(),
            open: close - noise * 0.5,
            high: close + base_price * 0.005,
            low: close - base_price * 0.005,
            close,
            volume: 100_000.0 + (i as f64) * 500.0,
        });
    }
    history.insert(symbol.to_string(), bars);
}

async fn seed_aggregated_signal(state: &SharedState, conviction: f64) {
    use tredo_core::agent::SkillDirection;
    use tredo_core::skill_aggregator::AggregatedSignal;
    let agg = AggregatedSignal {
        net_signal: conviction - 0.3,
        bullish_strength: conviction,
        bearish_strength: 1.0 - conviction,
        conviction,
        consensus: Some(if conviction > 0.6 {
            SkillDirection::Bullish
        } else if conviction < 0.4 {
            SkillDirection::Bearish
        } else {
            SkillDirection::Neutral
        }),
        participating_count: 5,
        bullish_count: if conviction > 0.5 { 4 } else { 1 },
        bearish_count: if conviction < 0.5 { 4 } else { 1 },
        neutral_count: 0,
    };
    *state.last_aggregated_signal.write().await = Some(agg);
}

/// Categorize a pipeline summary reason into a specific blocking point label.
fn categorize_block_point(reason: &str, executed: bool) -> &'static str {
    if executed {
        return "EXECUTED";
    }

    let lower = reason.to_lowercase();

    if lower.contains("no observable market data") {
        "BLOCK: No market data"
    } else if lower.contains("already have an open position") {
        "BLOCK: Phase 0 – Already open"
    } else if lower.contains("trading is disabled") || lower.contains("trading_enabled") {
        "BLOCK: Critical – Trading disabled"
    } else if lower.contains("daily drawdown") || lower.contains("drawdown") {
        "BLOCK: Critical – Drawdown > 2%"
    } else if lower.contains("red folder") || lower.contains("high-impact") {
        "BLOCK: Critical – Red folder event"
    } else if lower.contains("session timing")
        || lower.contains("session_timing")
        || lower.contains("outside allowed trading")
    {
        "BLOCK: Critical – Session timing"
    } else if lower.contains("portfolio heat")
        || lower.contains("portfolio_heat")
        || lower.contains("heat limit")
    {
        "BLOCK: High – Portfolio heat > 10%"
    } else if lower.contains("circuit breaker")
        || lower.contains("loss_circuit_breaker")
        || lower.contains("consecutive losses")
    {
        "BLOCK: High – 4+ consecutive losses"
    } else if lower.contains("max daily trades")
        || lower.contains("max_daily_trades")
        || lower.contains("8-trade")
    {
        "BLOCK: High – Max daily trades (8)"
    } else if lower.contains("cooldown") {
        "BLOCK: High – Trade cooldown active"
    } else if lower.contains("regime safety")
        || lower.contains("regime_safety")
        || lower.contains("bear regime")
    {
        "BLOCK: Medium – Bear regime safety"
    } else if lower.contains("confluence below")
        || lower.contains("confluence_minimum")
        || lower.contains("confluence")
    {
        "BLOCK: Medium – Confluence below threshold"
    } else if lower.contains("wfa gate")
        || lower.contains("wfa_gate")
        || lower.contains("inconsistent with recent")
    {
        "BLOCK: WFA Gate – Regime inconsistency"
    } else if lower.contains("debate layer") || lower.contains("debate") {
        "HOLD: Debate layer – No signal"
    } else if lower.contains("execution") || lower.contains("failed") {
        "ERROR: Execution failure"
    } else if lower.contains("hard rules gate blocked") {
        // Gate blocked but didn't match specific rule — extract the rule name from reason
        if lower.contains("critical") {
            "BLOCK: Critical (unspecified)"
        } else if lower.contains("high") {
            "BLOCK: High (unspecified)"
        } else if lower.contains("medium") {
            "BLOCK: Medium (unspecified)"
        } else {
            "BLOCK: Gate (unspecified)"
        }
    } else if lower.contains("hold") {
        "HOLD: No trade signal"
    } else {
        "BLOCK: Unknown reason"
    }
}

/// Simulate losing trades to realistically accumulate state (consecutive losses, drawdown, etc.)
async fn simulate_losing_trades(orch: &AutonomousOrchestrator, count: usize, symbols: &[&str]) {
    for i in 0..count {
        let symbol = symbols[i % symbols.len()];
        let entry = match symbol {
            "BTC" => 65_000.0,
            "ETH" => 3_500.0,
            "SOL" => 180.0,
            _ => 100.0,
        };

        // Add a losing position directly
        {
            let mut portfolio = orch.state.portfolio.write().await;
            let loss = -(entry * 0.02); // 2% loss per trade
            portfolio.open_positions.push(OpenPosition {
                symbol: symbol.to_string(),
                direction: TradeDirection::Long,
                entry_price: entry,
                current_price: entry * 0.98,
                stop_loss: entry * 0.97,
                take_profit: entry * 1.03,
                quantity: 1.0,
                unrealized_pnl: loss,
                unrealized_pnl_pct: -2.0,
                entry_time: Utc::now() - chrono::Duration::hours(1),
                risk_amount: entry * 0.02,
            });
            portfolio.total_trades_today += 1;
            portfolio.losing_trades_today += 1;
            portfolio.consecutive_losses += 1;
            portfolio.daily_pnl += loss;
            portfolio.max_drawdown_today = (portfolio.daily_pnl / portfolio.total_equity).abs();
        }

        // Close the losing position
        let _ = orch.portfolio.close_position(symbol, entry * 0.98).await;
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// TEST: Run 100 simulated trades and log which blocking point each hits
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn debug_100_trades_blocking_points() {
    let (orch, db_path) = setup_env("100_trades").await;

    // Seed rich OHLCV data for multiple symbols
    seed_ohlcv(&orch.state, "BTC", 65_000.0).await;
    seed_ohlcv(&orch.state, "ETH", 3_500.0).await;
    seed_ohlcv(&orch.state, "SOL", 180.0).await;

    // Seed high confluence so initial trades pass the gate
    seed_aggregated_signal(&orch.state, 0.85).await;
    *orch.state.market_regime.write().await = Some(MarketRegime::TrendingBull);

    let symbols = ["BTC", "ETH", "SOL"];
    let regimes = [
        Some(MarketRegime::TrendingBull),
        Some(MarketRegime::TrendingBear),
        Some(MarketRegime::Ranging),
        Some(MarketRegime::Volatile),
        Some(MarketRegime::LowLiquidity),
        None, // Unknown
    ];

    let mut results: Vec<(usize, String, &'static str)> = Vec::with_capacity(100);
    let mut blocking_counts: HashMap<&'static str, u32> = HashMap::new();
    let mut trades_executed: u32 = 0;
    let mut trades_blocked: u32 = 0;
    let mut trades_held: u32 = 0;
    let mut trades_errored: u32 = 0;

    println!("\n╔══ DEBUG: 100 TRADES PIPELINE WALKTHROUGH ══╗\n");

    for i in 0..100_usize {
        let symbol = symbols[i % 3];

        // ── Phase every 10 trades: change regime to trigger different rules ──
        if i > 0 && i % 10 == 0 {
            let regime_idx = (i / 10) % regimes.len();
            let new_regime = regimes[regime_idx];
            *orch.state.market_regime.write().await = new_regime;
            println!(
                "  [Phase] Trade {}: Regime changed to {:?}",
                i + 1,
                new_regime
            );
        }

        // ── Phase every 5 trades: vary confluence ──
        if i % 5 == 0 {
            // Vary from 0.30 to 0.85 to hit different confluence thresholds
            let conf = 0.30 + ((i / 5) as f64 * 0.03) % 0.60;
            seed_aggregated_signal(&orch.state, conf.min(0.85)).await;
        }

        // ── Every 25 trades: simulate accumulated losses ──
        if i > 0 && i % 25 == 0 {
            // Simulate 3 losing trades to trigger consecutive_losses circuit breaker
            simulate_losing_trades(&orch, 3, &symbols).await;
            println!("  [Phase] Trade {}: Injected 3 simulated losses", i + 1);
        }

        // ── Every 40 trades: reset portfolio to allow more trades through ──
        if i == 40 || i == 80 {
            {
                let mut portfolio = orch.state.portfolio.write().await;
                portfolio.consecutive_losses = 0;
                portfolio.total_trades_today = 0;
                portfolio.max_drawdown_today = 0.0;
                portfolio.daily_pnl = 0.0;
                portfolio.trading_enabled = true;
                portfolio.last_trade_time = None;
            }
            println!(
                "  [Phase] Trade {}: Reset portfolio counters to continue testing",
                i + 1
            );
        }

        // ── Run the pipeline ──
        let result = orch.run_full_pipeline(symbol).await;

        let (block_point, executed) = match &result {
            Ok(summary) => {
                let bp = categorize_block_point(&summary.reason, summary.executed);
                (bp, summary.executed)
            }
            Err(e) => (
                categorize_block_point(&format!("ERROR: {}", e), false),
                false,
            ),
        };

        // Track counts
        *blocking_counts.entry(block_point).or_insert(0) += 1;
        if executed {
            trades_executed += 1;
        } else if block_point.starts_with("BLOCK") {
            trades_blocked += 1;
        } else if block_point.starts_with("HOLD") {
            trades_held += 1;
        } else {
            trades_errored += 1;
        }

        results.push((i + 1, symbol.to_string(), block_point));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // PRINT SUMMARY TABLE
    // ═══════════════════════════════════════════════════════════════════════
    println!("\n╔═══════════════════════════════════════════════════════════════╗");
    println!("║      100 TRADES — BLOCKING POINT DISTRIBUTION                ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    println!("  {:<10} Count", "Category");
    println!("  {}", "-".repeat(50));
    println!("  {:<10} {:>5}", "EXECUTED", trades_executed);
    println!("  {:<10} {:>5}", "BLOCKED", trades_blocked);
    println!("  {:<10} {:>5}", "HOLD", trades_held);
    println!("  {:<10} {:>5}", "ERROR", trades_errored);
    println!("  {}", "-".repeat(50));
    println!("  {:<10} {:>5}", "TOTAL", 100);
    println!();

    println!(
        "  {:<50} {:>8} {:>10}",
        "Blocking Point", "Count", "Percent"
    );
    println!("  {}", "-".repeat(70));

    let mut sorted: Vec<(&&str, &u32)> = blocking_counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));

    for (point, count) in &sorted {
        let pct = **count as f64 / 100.0 * 100.0;
        println!("  {:<50} {:>8} {:>9.1}%", point, count, pct);
    }
    println!("  {}", "-".repeat(70));
    println!("  {:<50} {:>8} {:>9.1}%", "TOTAL", 100, 100.0);
    println!();

    // ═══════════════════════════════════════════════════════════════════════
    // PRINT DETAILED TRADE LOG
    // ═══════════════════════════════════════════════════════════════════════
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║                  DETAILED TRADE LOG                         ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    println!("  {:<6} {:<6} {:<55}", "Trade", "Symbol", "Blocking Point");
    println!("  {}", "-".repeat(70));

    for (num, symbol, point) in &results {
        let marker = if point.starts_with("EXEC") {
            "✅"
        } else if point.starts_with("BLOCK") {
            "⛔"
        } else if point.starts_with("HOLD") {
            "🔶"
        } else {
            "❌"
        };
        println!(
            "  {:<6} {:<6} {} {:<50}",
            format!("#{}", num),
            symbol,
            marker,
            point
        );
    }
    println!();

    // ═══════════════════════════════════════════════════════════════════════
    // KEY INSIGHTS
    // ═══════════════════════════════════════════════════════════════════════
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║                  KEY INTERPRETATION                         ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    let top_3: Vec<_> = sorted.iter().take(3).collect();
    println!("  Top blocking points:");
    for (point, count) in &top_3 {
        let pct = **count as f64 / 100.0 * 100.0;
        println!("    • {} ({} trades, {:.1}%)", point, count, pct);
    }

    if trades_executed > 0 {
        println!(
            "\n  {:.0}% of trades executed successfully ({} of 100).",
            trades_executed as f64, trades_executed
        );
    }
    if trades_blocked > 0 {
        println!(
            "  {:.0}% blocked by HardRulesGate rules ({} of 100).",
            trades_blocked as f64, trades_blocked
        );
    }
    if trades_held > 0 {
        println!(
            "  {:.0}% held by debate layer ({} of 100).",
            trades_held as f64, trades_held
        );
    }

    println!("\n  TIP: To debug a specific blocking point, search for the");
    println!("       block point name above and trace the pipeline logic in:");
    println!("       - crates/tredo-autonomous/src/hard_rules_gate.rs");
    println!("       - crates/tredo-autonomous/src/debate_layer.rs");
    println!("       - crates/tredo-autonomous/src/orchestrator_pipeline.rs");

    let _ = fs::remove_file(&db_path);
}
