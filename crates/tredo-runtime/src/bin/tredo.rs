//! tredo — Unified CLI for the autonomous agentic trading system.
//!
//! Usage:
//!     tredo --mode paper                     # Safe paper trading (default)
//!     tredo --mode live --confirm-live       # Live trading (requires explicit flag)
//!     tredo --mode backtest --data ./data.csv --capital 100000
//!     tredo --mode validate --cycles 100
//!     tredo --mode research                  # Observe market, no trading
//!
//! All modes share the same agent core, so backtested strategies = live strategies.

use anyhow::Context;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use tredo_autonomous::AutonomousOrchestrator;
use tredo_core::paper_engine::{BrokerRegistry, PaperEngineConfig};
use tredo_runtime::broker::{BrokerConfig, BrokerPluginManager};
use tredo_runtime::engine::RuntimeEngine;
use tredo_runtime::mode::{ModeConfig, TradingMode};

#[derive(Parser, Debug)]
#[command(name = "tredo", version, about = "Autonomous agentic trading system")]
struct Args {
    #[command(subcommand)]
    command: Option<BrokerCommand>,

    /// Trading mode
    #[arg(long, default_value_t = TradingMode::Paper)]
    mode: TradingMode,

    /// REQUIRED for live mode: explicit confirmation
    #[arg(long, default_value_t = false)]
    confirm_live: bool,

    /// Required for backtest mode: path to CSV (timestamp,open,high,low,close,volume)
    #[arg(long)]
    data: Option<String>,

    /// Required for validate mode: number of cycles
    #[arg(long, default_value_t = 50)]
    cycles: usize,

    /// For validate mode: induce regret to force rule adaptation
    #[arg(long, default_value_t = false)]
    induce_regret: bool,

    /// Max daily loss in currency (default 1000)
    #[arg(long, default_value_t = 1000.0)]
    max_daily_loss: f64,

    /// Starting capital for backtest (default 100000)
    #[arg(long, default_value_t = 100_000.0)]
    capital: f64,
}

#[derive(Subcommand, Debug)]
enum BrokerCommand {
    /// List available brokers and their config schemas
    List,
    /// Configure a broker interactively (e.g., `tredo configure zerodha`)
    Configure {
        /// Broker ID (e.g., "zerodha", "paper")
        broker_id: String,
    },
    /// Show policy cache health and top performers
    Cache,
}

// ── Subcommand Handlers ──────────────────────────────────────────────

/// Handle broker subcommands (list, configure, cache).
async fn handle_broker_command(cmd: &BrokerCommand) -> anyhow::Result<()> {
    let registry = BrokerPluginManager::new();

    match cmd {
        BrokerCommand::List => {
            println!("\nAvailable brokers:");
            for p in registry.list() {
                println!("  {} — {}", p.id, p.display_name);
                if !p.description.is_empty() {
                    println!("    {}", p.description);
                }
                for field in &p.config_schema {
                    let sensitive = if field.sensitive { " (sensitive)" } else { "" };
                    let default = field.default.as_deref().unwrap_or("(required)");
                    println!("    {} [{}]: {}{}", field.key, default, field.label, sensitive);
                }
                println!();
            }
        }
        BrokerCommand::Configure { broker_id } => {
            let plugin = registry
                .get(broker_id)
                .ok_or_else(|| anyhow::anyhow!("Unknown broker: {}", broker_id))?;

            let mut config = BrokerConfig::default();
            println!("\nConfiguring {} ({})", plugin.display_name, plugin.id);

            for field in &plugin.config_schema {
                let prompt = if field.sensitive {
                    format!("  {} (hidden, or set via env var): ", field.label)
                } else {
                    format!(
                        "  {} [{}]: ",
                        field.label,
                        field.default.as_deref().unwrap_or("")
                    )
                };
                print!("{}", prompt);
                use std::io::{self, Write};
                io::stdout().flush().ok();
                let mut input = String::new();
                io::stdin().read_line(&mut input)
                    .with_context(|| "Failed to read input")?;
                let value = input.trim();
                if !value.is_empty() {
                    config.set(&field.key, value);
                } else if let Some(default) = &field.default {
                    config.set(&field.key, default);
                }
            }

            // Save config
            registry.save_config(broker_id, &config)
                .map_err(|e| anyhow::anyhow!("Failed to save config for {}: {}", broker_id, e))?;
            println!("Configuration saved to ~/.tredo/{}.toml", broker_id);

            // Test connection
            println!("Testing connection...");
            match registry.instantiate(broker_id, &config).await {
                Ok(handle) => {
                    println!("✓ {} connected successfully", handle.plugin.display_name);
                }
                Err(e) => {
                    eprintln!("⚠ Connection failed: {}", e);
                    eprintln!("  Config was saved — fix credentials and run again.");
                }
            }
        }
        BrokerCommand::Cache => {
            let state = tredo_autonomous::state::SharedState::new(
                tredo_core::MemoryStore::new("tredo.redb")?,
                tredo_core::DisciplineRules::default(),
                tredo_core::Config::default(),
                "tredo_history.db",
            )?;
            let cache = tredo_runtime::policy_cache::PolicyCache::from_disk(state);

            println!("\nPolicy Cache Health");
            println!("  Entries: {}", cache.size());
            println!("  Total samples: {}", cache.total_samples());

            let top = cache.top_performers(3, 10);
            if top.is_empty() {
                println!("  No entries with \u{2265}3 samples yet.");
                println!("  Run paper trades to populate the cache.");
            } else {
                println!("\n  Top performers (min 3 samples):");
                for e in &top {
                    println!(
                        "    {} \u{2192} {:?} | WR={:.0}% n={} conf={:.2} regret={:.3}",
                        e.features.symbol,
                        e.recommended_action,
                        e.win_rate() * 100.0,
                        e.sample_size,
                        e.confidence(),
                        e.avg_regret
                    );
                }
            }

            // Show config thresholds
            println!("\n  Thresholds:");
            println!("    min_samples: {}", cache.config().min_samples);
            println!("    min_win_rate: {:.0}%", cache.config().min_win_rate * 100.0);
            println!("    min_confidence: {:.2}", cache.config().min_confidence);
        }
    }

    Ok(())
}

/// Try to build a live broker registry from saved config files.
/// Checks `~/.tredo/{alpaca,zerodha}.toml` and registers the first found.
async fn build_live_broker_registry() -> anyhow::Result<Option<BrokerRegistry>> {
    let registry = BrokerPluginManager::new();
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let tredo_dir = home.join(".tredo");

    // Check for saved broker configs in priority order: alpaca, zerodha
    let broker_ids = ["alpaca", "zerodha"];
    for id in &broker_ids {
        let config_path = tredo_dir.join(format!("{}.toml", id));
        if !config_path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: Failed to read {}: {}", config_path.display(), e);
                continue;
            }
        };

        let values: std::collections::HashMap<String, String> = match toml::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: Failed to parse {}: {}", config_path.display(), e);
                continue;
            }
        };

        let mut config = BrokerConfig::default();
        for (k, v) in &values {
            config.set(k, v);
        }

        match registry.instantiate(id, &config).await {
            Ok(handle) => {
                let br = BrokerRegistry::new(PaperEngineConfig::default());
                br.register_live_broker(std::sync::Arc::from(handle.adapter)).await;
                br.set_mode(tredo_core::paper_engine::TradingMode::Live).await
                    .map_err(|e| anyhow::anyhow!("Failed to set live mode: {}", e))?;
                return Ok(Some(br));
            }
            Err(e) => {
                eprintln!("Warning: Failed to instantiate broker '{}': {}", id, e);
                continue;
            }
        }
    }

    eprintln!("No saved broker config found. Use `tredo configure <broker_id>` first.");
    Ok(None)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    // Handle subcommands first (exit early if one was provided)
    if let Some(ref cmd) = args.command {
        return handle_broker_command(cmd).await;
    }

    // === SAFETY: live mode requires explicit confirmation ===
    if args.mode == TradingMode::Live && !args.confirm_live {
        eprintln!("\n╔══════════════════════════════════════════════════════════╗");
        eprintln!("║  ⚠ LIVE TRADING REQUESTED BUT NOT CONFIRMED              ║");
        eprintln!("║  You must pass --confirm-live to trade with real money.  ║");
        eprintln!("║  Run with --mode paper for safe paper trading.            ║");
        eprintln!("╚══════════════════════════════════════════════════════════╝\n");
        std::process::exit(1);
    }

    if args.mode == TradingMode::Backtest && args.data.is_none() {
        eprintln!("Error: --data <csv_path> is required for backtest mode");
        std::process::exit(1);
    }

    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║           tredo — Trading Real-time Edge Decision        ║");
    println!("║                  Optimisation (v3.0)                    ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("Mode: {}", args.mode);

    // === Initialize the system ===
    let state = tredo_autonomous::state::SharedState::new(
        tredo_core::MemoryStore::new("tredo.redb")?,
        tredo_core::DisciplineRules::default(),
        tredo_core::Config::default(),
        "tredo_history.db",
    )?;
    let mut orchestrator = AutonomousOrchestrator::new(state);
    orchestrator.init_tredo();

    // Get symbols
    let symbols = orchestrator.state.watchlist.read().await.clone();
    if symbols.is_empty() {
        eprintln!("Warning: watchlist is empty. Add symbols first or set WATCHLIST env var.");
    }

    // === Build mode config ===
    let mode_config = ModeConfig {
        mode: args.mode,
        require_trade_confirmation: true,
        max_daily_loss: args.max_daily_loss,
        symbol_whitelist: None,
        backtest_start: None,
        backtest_end: None,
        backtest_data_path: args.data,
        backtest_initial_capital: args.capital,
        validate_cycles: args.cycles,
        induce_regret: args.induce_regret,
    };

    // === Build broker registry (for live mode, loads saved config) ===
    let broker_registry: Option<Arc<BrokerRegistry>> = if args.mode == TradingMode::Live {
        match build_live_broker_registry().await {
            Ok(Some(registry)) => {
                println!("✓ Live broker registered: {}", registry.current_broker_name().await);
                Some(Arc::new(registry))
            }
            Ok(None) => {
                eprintln!("⚠ No live broker configured. Use `tredo configure <broker_id>` first.");
                None
            }
            Err(e) => {
                eprintln!("⚠ Failed to configure live broker: {}", e);
                eprintln!("  Falling back to paper mode for execution.");
                None
            }
        }
    } else {
        None
    };

    // === Run ===
    let engine = RuntimeEngine::new(mode_config, orchestrator, symbols, broker_registry).await?;
    let summary = engine.run().await?;

    println!("\n=== RUN COMPLETE ===");
    println!("Mode: {}", summary.mode);
    println!("Cycles: {}", summary.cycles_completed);
    println!("Events: {}", summary.events_processed);
    println!("Trades: {}", summary.trades_executed);
    println!("Cache hits: {} (Ollama calls: {})", summary.cache_hits, summary.ollama_calls);
    println!("P&L: ₹{:.2}", summary.total_pnl);
    println!("Max DD: {:.2}%", summary.max_drawdown * 100.0);
    println!("Duration: {}s", summary.duration_secs);

    Ok(())
}
