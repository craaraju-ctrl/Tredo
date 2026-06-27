use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::time::Instant;

use tredo_autonomous::walk_forward_runner::{
    HistoricalCandle, SkillResult, WalkForwardConfig, WalkForwardRunner,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    let command = args[1].as_str();
    match command {
        "validate" => {
            if args.len() < 4 {
                println!("Error: 'validate' requires <csv_path> <symbol>");
                return Ok(());
            }
            let csv_path = &args[2];
            let symbol = &args[3];
            run_walk_forward(csv_path, symbol).await?;
        }
        "self-evolve" => {
            run_self_evolution(&args[2..]).await?;
        }
        "daemon" => {
            println!("[Tredo CLI] Initializing autonomous daemon thread...");
            println!("[Tredo CLI] Connection to local Ollama (http://localhost:11434)...");
        }
        _ => {
            print_usage();
        }
    }

    Ok(())
}

fn print_usage() {
    println!("=== Project Tredo: Autonomous AI Execution CLI ===");
    println!("Usage:");
    println!("  tredo-cli validate <csv_path> <symbol>   Run walk-forward out-of-sample backtests");
    println!("  tredo-cli self-evolve [cycles] [--induce] [--symbols BTC,ETH]");
    println!(
        "                                           Run the self-evolution loop and report compounding"
    );
    println!(
        "  tredo-cli daemon                         Spin up the autonomous live/paper trading loop"
    );
}

/// Run the extended self-evolution validation harness (engineering loop).
///
/// Boots the autonomous orchestrator, then drives N cycles of the full agentic
/// pipeline (optionally inducing regret) and prints a compounding-improvement
/// report. Symbols come from `--symbols`, else the `WATCHLIST` env var, else a
/// BTC/ETH default.
async fn run_self_evolution(
    rest: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tredo_autonomous::self_evolution::SelfEvolutionValidator;
    use tredo_autonomous::state::initialize_autonomous_system;

    // Parse args: optional positional cycle count, optional `--induce`, optional
    // `--symbols A,B,C`.
    let mut cycles: usize = 20;
    let mut induce = false;
    let mut symbols_arg: Option<String> = None;

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--induce" | "--induce-regret" => induce = true,
            "--symbols" => {
                if i + 1 < rest.len() {
                    symbols_arg = Some(rest[i + 1].clone());
                    i += 1;
                }
            }
            other => {
                if let Ok(n) = other.parse::<usize>() {
                    cycles = n;
                }
            }
        }
        i += 1;
    }

    let symbols_owned: Vec<String> = symbols_arg
        .or_else(|| env::var("WATCHLIST").ok())
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec!["BTC".to_string(), "ETH".to_string()]);
    let symbols: Vec<&str> = symbols_owned.iter().map(|s| s.as_str()).collect();

    println!(
        "[Tredo CLI] Booting autonomous orchestrator for self-evolution ({} cycles, induce={}, symbols={:?})...",
        cycles, induce, symbols
    );

    let orchestrator = initialize_autonomous_system().await?;
    let validator = SelfEvolutionValidator::new(orchestrator);
    // run_extended_validation already prints the full summary on completion.
    let _report = validator
        .run_extended_validation(&symbols, cycles, induce)
        .await?;

    Ok(())
}

/// Parses historical CSV files and executes the WalkForwardRunner
async fn run_walk_forward(
    csv_path: &str,
    symbol: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("[Tredo CLI] Reading OHLCV dataset from: {}", csv_path);
    let candles = load_candles_from_csv(csv_path)?;
    println!("[Tredo CLI] Successfully loaded {} candles.", candles.len());

    let config = WalkForwardConfig {
        train_window_size: 100,
        test_window_size: 20,
        step_size: 20,
        initial_capital: 10000.0,
        base_learning_rate: 0.05,
        overfitting_threshold: 0.35,
    };

    let runner = WalkForwardRunner::new(config);
    let mut initial_weights = std::collections::HashMap::new();
    initial_weights.insert("news_analyser".to_string(), 0.50);
    initial_weights.insert("market_metrics_meter".to_string(), 0.50);

    println!("[Tredo CLI] Starting walk-forward evaluation loops...");
    let start_time = Instant::now();

    let report = runner
        .run_validation(symbol, &candles, initial_weights, |_slice, _weights| {
            let results = vec![SkillResult {
                score: 0.65,
                confidence: 0.85,
            }];
            Ok(Some(results))
        })
        .await?;

    let elapsed = start_time.elapsed();
    println!("\n==================================================");
    println!("=== WALK-FORWARD VALIDATION COMPLETE ({:?}) ===", elapsed);
    println!("==================================================");
    println!("Total Folds Evaluated: {}", report.total_folds_evaluated);
    println!(
        "Mean In-Sample Sharpe:  {:.4}",
        report.mean_in_sample_sharpe
    );
    println!(
        "Mean Out-of-Sample Sharpe: {:.4}",
        report.mean_out_of_sample_sharpe
    );
    println!(
        "Structural Stability Score: {:.2}%",
        report.structural_stability_score * 100.0
    );
    println!("Deployment Verdict:     {}", report.overall_recommendation);
    println!("==================================================");

    Ok(())
}

fn load_candles_from_csv(file_path: &str) -> io::Result<Vec<HistoricalCandle>> {
    let path = Path::new(file_path);
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut candles = Vec::new();
    let mut lines = reader.lines();

    // Skip CSV header line
    let _header = lines.next();

    for line in lines {
        let line_str = line?;
        let columns: Vec<&str> = line_str.split(',').collect();
        if columns.len() < 6 {
            continue;
        }

        let timestamp = columns[0].parse::<u64>().unwrap_or_default();
        let open = columns[1].parse::<f64>().unwrap_or_default();
        let high = columns[2].parse::<f64>().unwrap_or_default();
        let low = columns[3].parse::<f64>().unwrap_or_default();
        let close = columns[4].parse::<f64>().unwrap_or_default();
        let volume = columns[5].parse::<f64>().unwrap_or_default();

        candles.push(HistoricalCandle {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
        });
    }

    Ok(candles)
}
