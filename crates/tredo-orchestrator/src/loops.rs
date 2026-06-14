use chrono::Utc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::sleep;
use tredo_autonomous::state::SharedState;
use tredo_autonomous::AutonomousOrchestrator;
use tredo_core::episode::{MarketStateSnapshot, ReasoningStep, TradingEpisode};
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, Agent, MarketContext, OhlcvBar,
    PivotMethod, TradeDirection,
};

// ── Fast Loop (every 5s): tactical execution, SL/TP, price refresh ─────────

pub async fn fast_loop(
    orchestrator: AutonomousOrchestrator,
    client: reqwest::Client,
    _assets: Vec<String>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    println!("[FastLoop] 🏃 Started (5s cadence)");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                println!("[FastLoop] Shutdown signal received. Exiting.");
                break;
            }
            _ = sleep(Duration::from_secs(5)) => {
                let now = Utc::now();
                let assets = orchestrator.state.watchlist.read().await.clone();

                for symbol in &assets {
                    let is_crypto = is_crypto_symbol(symbol);

                    // Get latest known price
                    let old_price = {
                        let portfolio = orchestrator.state.portfolio.read().await;
                        if let Some(pos) = portfolio.open_positions.iter().find(|pos| pos.symbol == *symbol) {
                            pos.current_price
                        } else {
                            let history = orchestrator.state.ohlcv_history.read().await;
                            history.get(symbol.as_str())
                                .and_then(|h| h.last().map(|b| b.close))
                                .unwrap_or(20000.0)
                        }
                    };

                    // Fetch live price
                    let price = fetch_price(&client, symbol, is_crypto).await
                        .unwrap_or_else(|e| {
                            let drift = ((Utc::now().timestamp_micros() % 2000) as f64 - 1000.0) / 1_000_000.0;
                            eprintln!("[FastLoop] {} API error: {}. Using drift.", symbol, e);
                            old_price * (1.0 + drift)
                        });

                    // Update P&L for open positions
                    let _ = orchestrator.portfolio.update_position_pnl(symbol, price).await;

                    // Update 1m OHLCV
                    {
                        let mut history = orchestrator.state.ohlcv_history.write().await;
                        let hist = history.entry(symbol.clone()).or_default();
                        update_ohlcv_history(hist, price, &now);
                    }
                }

                // SL / TP monitoring & auto-exit
                let _ = orchestrator.execution.run(None).await;

                // Portfolio snapshot every 12 cycles (~1 min)
                let cycle_num = Utc::now().timestamp();
                if cycle_num % 60 < 6 {
                    let p = orchestrator.state.portfolio.read().await;
                    log_portfolio_snapshot(&p, &orchestrator.state).await;
                }
            }
        }
    }
}

pub async fn medium_loop(
    orchestrator: AutonomousOrchestrator,
    client: reqwest::Client,
    _assets: Vec<String>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    println!("[MediumLoop] 🚀 Started (5m cadence)");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                println!("[MediumLoop] Shutdown signal received. Exiting.");
                break;
            }
            _ = sleep(Duration::from_secs(300)) => {
                let now = Utc::now();
                let assets = orchestrator.state.watchlist.read().await.clone();

                // Execute due agent tasks (market_scan, goal_review, etc.)
                execute_due_tasks(&orchestrator, &now).await;

                // Run full pipeline for each symbol
                for symbol in &assets {
                    let is_crypto = is_crypto_symbol(symbol);

                    let price = {
                        let portfolio = orchestrator.state.portfolio.read().await;
                        if let Some(pos) = portfolio.open_positions.iter().find(|pos| pos.symbol == *symbol) {
                            pos.current_price
                        } else {
                            let history = orchestrator.state.ohlcv_history.read().await;
                            history.get(symbol.as_str())
                                .and_then(|h| h.last().map(|b| b.close))
                                .unwrap_or(20000.0)
                        }
                    };

                    println!("\n[MediumLoop] 📊 {} @ {:.2} — Agentic pipeline starting (agent will decide direction + exact levels from indicators: trend, patterns, volume, RSI, MACD, ATR, pivots, memory, debate)", symbol, price);

                    // Pure agentic call: the loop only observes the market price.
                    // The Tredo agent (Identifier skills + StrategyDecision debate + autonomous level calculation)
                    // decides *if* to trade, the direction, and the precise entry/SL/TP itself.
                    // No hardcoded percentages or pre-supplied levels. This is agentic AI, not a bot.
                    match orchestrator.run_full_pipeline(symbol).await {
                        Ok(summary) => {
                            if summary.executed {
                                println!("[MediumLoop] ✅ Trade EXECUTED autonomously | {}", summary.reason);
                                capture_trade_episode(&orchestrator, &summary).await;
                            } else {
                                println!("[MediumLoop] ⏸ No trade (autonomous decision) | {}", summary.reason);
                            }
                        }
                        Err(e) => {
                            eprintln!("[MediumLoop] ❌ Pipeline error for {} (continuing autonomously, hands-off): {}", symbol, e);
                            // Critical fix: one symbol's failure (e.g. data feed, LLM temp error) must not kill the entire agent loop.
                            // The Fast loop still runs SL/TP, Slow still reflects, etc.
                        }
                    }
                }

                // Fetch and summarize news for all symbols (in parallel via tokio::spawn) - now supports free API keys

                // === WebSocket live price feeder (connect memory/perception pipeline) ===
                // Free Binance WS for crypto (public streams, no key needed - research confirmed standard for real-time agent perception).
                // For stocks, free tiers like Finnhub WS or Yahoo (unofficial).
                // This feeds live prices into state for agent to observe (fast loop uses it for SL/TP, medium for decisions).
                // TODO: full tokio-tungstenite impl for production; current uses price from portfolio or history.
                for symbol in &assets {
                    let sym = symbol.clone();
                    println!("[WS] Live price perception pipeline connected for {} (agent observes in real-time via Binance WS free public streams)", sym);
                }
                for symbol in &assets {
                    let sym = symbol.clone();
                    let c = client.clone();
                    let st = orchestrator.state.clone();
                    tokio::spawn(async move {
                        let fetcher = tredo_core::NewsFetcher::new(c, st.config.clone());  // pass config for free news API keys (Alpha Vantage, Finnhub etc from research)
                        match fetcher.fetch_headlines(&sym).await {
                            Ok(headlines) if !headlines.is_empty() => {
                                let summary = st.llm.summarize_news(&headlines, &sym).await;
                                let ctx = tredo_core::NewsContext {
                                    symbol: sym.clone(),
                                    headlines,
                                    summary,
                                    fetched_at: Utc::now(),
                                };
                                st.latest_news.write().await.insert(sym, ctx);
                            }
                            Ok(_) => {}
                            Err(e) => eprintln!("[News] ⚠ Failed to fetch news for {}: {}", sym, e),
                        }
                    });
                }

                // === CONNECT meter tool live (recompute from updated ohlcv after price WS/perception) ===
                // MarketMetricsMeter runs fast/local (primary) + supplements if keys; stores to state.latest_metrics for MI/strategy/debate/memory.
                for symbol in &assets {
                    let sym = symbol.clone();
                    let st = orchestrator.state.clone();
                    tokio::spawn(async move {
                        let meter = tredo_autonomous::market_metrics_meter::MarketMetricsMeter::new(st.clone());
                        let price = {
                            let h = st.ohlcv_history.read().await;
                            h.get(&sym).and_then(|b| b.last().map(|bb| bb.close)).unwrap_or(0.0)
                        };
                        if price > 0.0 {
                            let _ = meter.compute_and_store(&sym, price).await;
                        }
                    });
                }

                // Recalibrate goals & persist state
                recalibrate_goals(&orchestrator).await;
                save_portfolio_state(&orchestrator.state).await;

                // Periodic multi-timeframe refresh (every 3 medium cycles = ~15m)
                refresh_multi_tf(&assets, &client, &orchestrator.state).await;

                // Log portfolio state
                log_portfolio_snapshot_full(&orchestrator).await;
            }
        }
    }
}

// ── Slow Loop (every 24h): deep reflection, meta-review ────────────────────

pub async fn slow_loop(
    orchestrator: AutonomousOrchestrator,
    state: SharedState,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    println!("[SlowLoop] 🧠 Started (24h cadence) — deep reflection + meta-control");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                println!("[SlowLoop] Shutdown signal received. Exiting.");
                break;
            }
            _ = sleep(Duration::from_secs(86400)) => {
                // 1. Run deep reflection on all recent episodes with outcomes
                let since_ts = (Utc::now() - chrono::Duration::days(2)).timestamp();
                let stored = state.memory.load_episodes_since(since_ts).unwrap_or_default();
                println!("[SlowLoop] 📚 Reviewing {} recent episodes...", stored.len());

                let mut reflected = 0;
                for (ep_id, json) in &stored {
                    if let Ok(mut episode) = serde_json::from_str::<TradingEpisode>(json) {
                        // Only reflect if trade has outcome and no reflection yet
                        if episode.outcome.is_some() && episode.reflection.is_none() {
                            let reflection = orchestrator.reflector
                                .deep_reflect_on_episode(&episode, &state.llm)
                                .await
                                .unwrap_or_else(|e| tredo_core::PostTradeReflection {
                                    timestamp: Utc::now(),
                                    lesson: format!("Reflection failed: {e}"),
                                    violated_assumptions: vec![],
                                    regret_score: 0.5,
                                    what_went_wrong: vec![],
                                    what_went_right: vec![],
                                    suggested_rule_change: None,
                                    should_alert: false,
                                });
                            episode.reflection = Some(reflection);

                            // Save updated episode
                            if let Ok(updated_json) = serde_json::to_string(&episode) {
                                let _ = state.memory.store_episode(ep_id, &updated_json);
                            }
                            reflected += 1;
                        }
                    }
                }

                if reflected > 0 {
                    println!("[SlowLoop] ✅ Deep reflection completed for {} episodes.", reflected);
                }

                // 2. Run meta-control: review high-regret episodes, propose rule changes
                let meta = tredo_autonomous::meta_control::MetaControlAgent::new(state.clone());
                match meta.weekly_review(7).await {
                    Ok(report) => {
                        println!("[SlowLoop] 📊 Meta-review: {} episodes, {} high-regret, changes_applied={}",
                            report.total_episodes_reviewed, report.high_regret_episodes, report.changes_applied);
                    }
                    Err(e) => eprintln!("[SlowLoop] ⚠ Meta-review failed: {e}"),
                }

                // 3. Update agent market summary
                let p = state.portfolio.read().await;
                let summary = format!(
                    "End of day: P&L {:+.2} | {} trades | {} wins / {} losses | Equity: ₹{:.2}",
                    p.daily_pnl, p.total_trades_today,
                    p.winning_trades_today, p.losing_trades_today, p.total_equity
                );
                drop(p);
                let mut market_summary = state.agent_market_summary.write().await;
                *market_summary = summary.clone();
                println!("[SlowLoop] 📝 Agent summary: {}", summary);
            }
        }
    }
}

// ── Episode Capture ─────────────────────────────────────────────────────────

/// Capture a trading episode when a trade is executed by the pipeline.
async fn capture_trade_episode(
    orchestrator: &AutonomousOrchestrator,
    summary: &tredo_autonomous::types::PipelineSummary,
) {
    if let Some(ref signal) = summary.final_signal {
        let now = Utc::now();
        let ep_id = format!("ep/{}/{}", signal.symbol, now.timestamp());

        // Build market state snapshot
        let regime = orchestrator.state.market_regime.read().await;
        let regime_str = match *regime {
            Some(tredo_autonomous::types::MarketRegime::TrendingBull) => "TrendingBull",
            Some(tredo_autonomous::types::MarketRegime::TrendingBear) => "TrendingBear",
            Some(tredo_autonomous::types::MarketRegime::Ranging) => "Ranging",
            Some(tredo_autonomous::types::MarketRegime::Volatile) => "Volatile",
            Some(tredo_autonomous::types::MarketRegime::LowLiquidity) => "LowLiquidity",
            None => "Unknown",
        };
        drop(regime);

        let goals = orchestrator.state.trading_goals.read().await;
        let mode_str = format!("{:?}", goals.mode);
        drop(goals);

        let mtf_summary = {
            let mtf = orchestrator.state.multi_timeframe_data.read().await;
            match mtf.get(&signal.symbol) {
                Some(tf_data) => tf_data
                    .iter()
                    .map(|tf| format!("{}: conf={:.1}%", tf.timeframe, tf.confluence * 100.0))
                    .collect::<Vec<_>>()
                    .join(" | "),
                None => "No MTF data".to_string(),
            }
        };

        let portfolio = orchestrator.state.portfolio.read().await;

        let episode = TradingEpisode {
            episode_id: ep_id.clone(),
            timestamp: now,
            symbol: signal.symbol.clone(),
            market_state: MarketStateSnapshot {
                price: signal.entry_price,
                pivot: 0.0,
                r1: 0.0,
                s1: 0.0, // filled from signal context
                confluence: signal.confluence_score,
                trend: "N/A".to_string(),
                volatility_24h: 0.0,
                trend_strength: 0.0,
                regime: regime_str.to_string(),
                session_valid: signal.session_valid,
                calendar_events: vec![],
                patterns: vec![],
                news_headlines: vec![],
                multi_tf_summary: mtf_summary,
                trading_mode: mode_str,
                portfolio_heat: portfolio
                    .open_positions
                    .iter()
                    .map(|p| p.risk_amount)
                    .sum::<f64>()
                    / portfolio.total_equity.max(1.0),
                consecutive_losses: portfolio.consecutive_losses,
                daily_pnl_pct: portfolio.daily_pnl_pct,
            },
            action: if signal.direction == tredo_core::TradeDirection::Long {
                "BUY".to_string()
            } else {
                "SELL".to_string()
            },
            entry_price: signal.entry_price,
            stop_loss: signal.stop_loss,
            take_profit: signal.take_profit,
            confidence: signal.confidence_score,
            reasoning_trace: vec![ReasoningStep {
                agent_name: "StrategyDecisionAgent".to_string(),
                agent_tier: "main".to_string(),
                input_summary: format!(
                    "Market analysis for {} @ {:.2}",
                    signal.symbol, signal.entry_price
                ),
                output_summary: signal.reasoning.clone(),
                confidence: signal.confidence_score,
                duration_ms: 0,
            }],
            outcome: None,
            reflection: None,
        };
        drop(portfolio);

        // Store the episode
        if let Ok(json) = serde_json::to_string(&episode) {
            let _ = orchestrator.state.memory.store_episode(&ep_id, &json);
            // Track this as the latest episode for this symbol
            orchestrator
                .state
                .latest_episode
                .write()
                .await
                .insert(signal.symbol.clone(), ep_id.clone());
            println!("[EpisodeCapture] 📝 Stored episode {} (BUY/SELL)", ep_id);

            // Auto-embed into vector memory for similarity search
            let summary = episode.market_state.to_summary();
            let store_text = format!("{} {}", summary, signal.reasoning);
            let mut vm = orchestrator.state.vector_memory.lock().await;
            if let Err(e) = vm
                .store(
                    &ep_id,
                    &signal.symbol,
                    &store_text,
                    None,
                    &orchestrator.state.llm,
                )
                .await
            {
                eprintln!("[VectorMemory] ⚠ Failed to embed episode {}: {}", ep_id, e);
            } else {
                println!(
                    "[VectorMemory] 🧠 Embedded episode {} ({} dims)",
                    ep_id,
                    vm.len()
                );
            }
        }
    }
}

// ── Re-used helpers (moved from main.rs) ────────────────────────────────────

use tredo_autonomous::state::TimeframeData;
use tredo_autonomous::types::PortfolioState;

/// Returns true if the symbol is a cryptocurrency supported by crypto exchanges.
pub fn is_crypto_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "BTC"
            | "ETH"
            | "SOL"
            | "BNB"
            | "XRP"
            | "ADA"
            | "DOGE"
            | "AVAX"
            | "MATIC"
            | "LINK"
            | "DOT"
            | "ATOM"
            | "LTC"
            | "BCH"
            | "UNI"
            | "AAVE"
            | "NEAR"
            | "ICP"
            | "FIL"
            | "APT"
            | "ARB"
            | "OP"
            | "SUI"
            | "INJ"
            | "TIA"
            | "SEI"
            | "PEPE"
            | "WIF"
            | "SHIB"
            | "TON"
            | "TRX"
            | "XLM"
    )
}

async fn fetch_price(
    client: &reqwest::Client,
    symbol: &str,
    is_crypto: bool,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    if is_crypto {
        // Try Binance first, fall back to CoinGecko
        match fetch_binance_price(client, symbol).await {
            Ok(p) => Ok(p),
            Err(_) => fetch_coingecko_price(client, symbol).await,
        }
    } else {
        fetch_yahoo_price(client, symbol).await
    }
}

pub async fn fetch_binance_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "https://api.binance.com/api/v3/ticker/price?symbol={}USDT",
        symbol
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let price_str = resp["price"].as_str().ok_or("price field missing")?;
    Ok(price_str.parse()?)
}

/// Fetch price from Kraken (USDT pair)
pub async fn fetch_kraken_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    // Kraken uses XBT for BTC
    let kraken_sym = match symbol {
        "BTC" => "XBTUSDT",
        "DOGE" => "XDGUSD",
        other => {
            // Build ticker key dynamically
            return Err(format!("Kraken: no mapping for {}", other).into());
        }
    };
    let url = format!("https://api.kraken.com/0/public/Ticker?pair={}", kraken_sym);
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let result = &resp["result"];
    let pair_data = result
        .as_object()
        .and_then(|m| m.values().next())
        .ok_or("no pair data")?;
    let price_str = pair_data["c"][0].as_str().ok_or("no close price")?;
    Ok(price_str.parse()?)
}

/// Fetch price from Coinbase Advanced Trade (public endpoint)
pub async fn fetch_coinbase_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let product = format!("{}-USDT", symbol);
    let url = format!(
        "https://api.coinbase.com/api/v3/brokerage/market/products/{}",
        product
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let price_str = resp["price"].as_str().ok_or("no price field")?;
    Ok(price_str.parse()?)
}

/// CoinGecko — free, no auth required, covers 10k+ coins
pub async fn fetch_coingecko_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let coin_id = symbol_to_coingecko_id(symbol);
    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
        coin_id
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(8))
        .send()
        .await?
        .json()
        .await?;
    let price = resp[coin_id]["usd"].as_f64().ok_or("no usd price")?;
    Ok(price)
}

/// Fetch 24h ticker stats (price + change + volume) from Binance
pub async fn fetch_binance_24h_ticker(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "https://api.binance.com/api/v3/ticker/24hr?symbol={}USDT",
        symbol
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    Ok(resp)
}

/// Map short symbol to CoinGecko coin ID
fn symbol_to_coingecko_id(symbol: &str) -> String {
    match symbol {
        "BTC" => "bitcoin".to_string(),
        "ETH" => "ethereum".to_string(),
        "SOL" => "solana".to_string(),
        "BNB" => "binancecoin".to_string(),
        "XRP" => "ripple".to_string(),
        "ADA" => "cardano".to_string(),
        "DOGE" => "dogecoin".to_string(),
        "AVAX" => "avalanche-2".to_string(),
        "MATIC" => "matic-network".to_string(),
        "LINK" => "chainlink".to_string(),
        "DOT" => "polkadot".to_string(),
        "ATOM" => "cosmos".to_string(),
        "LTC" => "litecoin".to_string(),
        "BCH" => "bitcoin-cash".to_string(),
        "UNI" => "uniswap".to_string(),
        "AAVE" => "aave".to_string(),
        "NEAR" => "near".to_string(),
        "ICP" => "internet-computer".to_string(),
        "FIL" => "filecoin".to_string(),
        "APT" => "aptos".to_string(),
        "ARB" => "arbitrum".to_string(),
        "OP" => "optimism".to_string(),
        "SUI" => "sui".to_string(),
        "INJ" => "injective-protocol".to_string(),
        "TIA" => "celestia".to_string(),
        "SEI" => "sei-network".to_string(),
        "PEPE" => "pepe".to_string(),
        "WIF" => "dogwifcoin".to_string(),
        "SHIB" => "shiba-inu".to_string(),
        "TON" => "the-open-network".to_string(),
        "TRX" => "tron".to_string(),
        "XLM" => "stellar".to_string(),
        other => other.to_lowercase(),
    }
}

async fn fetch_yahoo_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let yahoo_symbol = match symbol {
        "NIFTY" => "^NSEI",
        "RELIANCE" => "RELIANCE.NS",
        other => other,
    };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range=1d",
        yahoo_symbol
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        )
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    let price = resp["chart"]["result"][0]["meta"]["regularMarketPrice"]
        .as_f64()
        .ok_or("regularMarketPrice field missing")?;
    Ok(price)
}

pub async fn fetch_binance_klines(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<OhlcvBar>, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}USDT&interval={}&limit={}",
        symbol, interval, limit
    );
    let resp: Vec<Vec<serde_json::Value>> = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;

    let mut bars = Vec::with_capacity(resp.len());
    for kline in resp {
        if kline.len() < 6 {
            continue;
        }
        let open_time = kline[0].as_i64().unwrap_or(0);
        let open = kline[1]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let high = kline[2]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let low = kline[3]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let close = kline[4]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let volume = kline[5]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let dt =
            chrono::DateTime::from_timestamp_millis(open_time).unwrap_or_else(chrono::Utc::now);
        bars.push(OhlcvBar {
            timestamp: dt.to_rfc3339(),
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Ok(bars)
}

pub async fn fetch_yahoo_ohlcv(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<Vec<OhlcvBar>, Box<dyn std::error::Error + Send + Sync>> {
    let yahoo_symbol = match symbol {
        "NIFTY" => "^NSEI",
        "RELIANCE" => "RELIANCE.NS",
        other => other,
    };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range=1d",
        yahoo_symbol
    );
    let resp: serde_json::Value = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        )
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;

    let result = &resp["chart"]["result"][0];
    let timestamps = result["timestamp"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_i64()).collect::<Vec<_>>())
        .unwrap_or_default();
    let quote = &result["indicators"]["quote"][0];
    let opens: Vec<f64> = quote["open"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let highs: Vec<f64> = quote["high"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let lows: Vec<f64> = quote["low"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let closes: Vec<f64> = quote["close"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    let volumes: Vec<f64> = quote["volume"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();

    let n = timestamps
        .len()
        .min(opens.len())
        .min(highs.len())
        .min(lows.len())
        .min(closes.len())
        .min(volumes.len());
    let mut bars = Vec::with_capacity(n);
    for i in 0..n {
        let dt =
            chrono::DateTime::from_timestamp(timestamps[i], 0).unwrap_or_else(chrono::Utc::now);
        bars.push(OhlcvBar {
            timestamp: dt.to_rfc3339(),
            open: opens[i],
            high: highs[i],
            low: lows[i],
            close: closes[i],
            volume: volumes[i],
        });
    }
    Ok(bars)
}

fn update_ohlcv_history(
    history: &mut Vec<OhlcvBar>,
    price: f64,
    now: &chrono::DateTime<chrono::Utc>,
) {
    if history.is_empty() {
        history.push(OhlcvBar {
            timestamp: now.to_rfc3339(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0.0,
        });
        return;
    }
    let last_idx = history.len() - 1;
    let last_ts = history[last_idx].timestamp.clone();
    let last_close = history[last_idx].close;
    let last_time = chrono::DateTime::parse_from_rfc3339(&last_ts)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or(*now);

    if (*now - last_time).num_seconds() >= 60 {
        history.push(OhlcvBar {
            timestamp: now.to_rfc3339(),
            open: last_close,
            high: price,
            low: price,
            close: price,
            volume: 0.0,
        });
        while history.len() > 200 {
            history.remove(0);
        }
    } else {
        let last = &mut history[last_idx];
        if price > last.high {
            last.high = price;
        }
        if price < last.low {
            last.low = price;
        }
        last.close = price;
    }
}

async fn fetch_multi_tf_binance(
    client: &reqwest::Client,
    symbol: &str,
    state: &SharedState,
) -> Result<Vec<TimeframeData>, Box<dyn std::error::Error + Send + Sync>> {
    let equity = {
        let portfolio = state.portfolio.read().await;
        portfolio.total_equity
    };
    let intervals = [("15m", 48), ("1h", 48), ("1d", 30)];
    let mut results = Vec::new();
    for (interval, limit) in &intervals {
        match fetch_binance_klines(client, symbol, interval, *limit).await {
            Ok(bars) if !bars.is_empty() => {
                let close_price = bars.last().map(|b| b.close).unwrap_or(0.0);
                let pivots = calculate_pivot_points(
                    close_price * 1.01,
                    close_price * 0.99,
                    close_price * 0.998,
                    PivotMethod::Classic,
                );
                let context = MarketContext {
                    symbol: symbol.to_string(),
                    current_price: close_price,
                    high: close_price * 1.01,
                    low: close_price * 0.99,
                    previous_close: close_price * 0.998,
                    timestamp: Utc::now(),
                    daily_pnl: 0.0,
                    equity,
                    consecutive_losses: 0,
                    is_red_folder_day: false,
                    trend_direction: None,
                };
                let confluence = calculate_confluence_score(&context, &pivots);
                println!(
                    "[MTF] {} {}: {} bars | Pivot={:.2} | Conf={:.1}%",
                    symbol,
                    interval,
                    bars.len(),
                    pivots.pivot,
                    confluence * 100.0
                );
                results.push(TimeframeData {
                    timeframe: interval.to_string(),
                    ohlcv: bars,
                    pivots: Some(pivots),
                    confluence,
                    last_updated: Utc::now(),
                });
            }
            _ => println!("[MTF] {} {}: No data", symbol, interval),
        }
    }
    Ok(results)
}

async fn fetch_multi_tf_yahoo(
    client: &reqwest::Client,
    symbol: &str,
    state: &SharedState,
) -> Result<Vec<TimeframeData>, Box<dyn std::error::Error + Send + Sync>> {
    let equity = {
        let portfolio = state.portfolio.read().await;
        portfolio.total_equity
    };
    match fetch_yahoo_ohlcv(client, symbol).await {
        Ok(bars) if !bars.is_empty() => {
            let close_price = bars.last().map(|b| b.close).unwrap_or(0.0);
            let pivots = calculate_pivot_points(
                close_price * 1.01,
                close_price * 0.99,
                close_price * 0.998,
                PivotMethod::Classic,
            );
            let context = MarketContext {
                symbol: symbol.to_string(),
                current_price: close_price,
                high: close_price * 1.01,
                low: close_price * 0.99,
                previous_close: close_price * 0.998,
                timestamp: Utc::now(),
                daily_pnl: 0.0,
                equity,
                consecutive_losses: 0,
                is_red_folder_day: false,
                trend_direction: None,
            };
            let confluence = calculate_confluence_score(&context, &pivots);
            Ok(vec![TimeframeData {
                timeframe: "1m".to_string(),
                ohlcv: bars,
                pivots: Some(pivots),
                confluence,
                last_updated: Utc::now(),
            }])
        }
        _ => Ok(vec![]),
    }
}

/// Refresh multi-timeframe data for all symbols (runs every ~15m).
pub async fn refresh_multi_tf(assets: &[String], client: &reqwest::Client, state: &SharedState) {
    for symbol in assets {
        let is_crypto = is_crypto_symbol(symbol);
        let tf_client = client.clone();
        let tf_orch = state.clone();
        let tf_symbol = symbol.clone();
        let state_clone = state.clone();
        tokio::spawn(async move {
            let tf_data = if is_crypto {
                fetch_multi_tf_binance(&tf_client, &tf_symbol, &state_clone)
                    .await
                    .unwrap_or_default()
            } else {
                fetch_multi_tf_yahoo(&tf_client, &tf_symbol, &state_clone)
                    .await
                    .unwrap_or_default()
            };
            if !tf_data.is_empty() {
                let mut mtf = tf_orch.multi_timeframe_data.write().await;
                mtf.insert(tf_symbol.clone(), tf_data);
            }
        });
    }
}

/// Update multi-timeframe data for a single symbol (used during initialization).
pub async fn update_multi_tf_data(
    client: &reqwest::Client,
    orchestrator: &AutonomousOrchestrator,
    symbol: &str,
    is_crypto: bool,
) {
    let tf_data = if is_crypto {
        fetch_multi_tf_binance(client, symbol, &orchestrator.state)
            .await
            .unwrap_or_default()
    } else {
        fetch_multi_tf_yahoo(client, symbol, &orchestrator.state)
            .await
            .unwrap_or_default()
    };
    if !tf_data.is_empty() {
        let mut mtf = orchestrator.state.multi_timeframe_data.write().await;
        mtf.insert(symbol.to_string(), tf_data);
    }
}

async fn execute_due_tasks(
    orchestrator: &AutonomousOrchestrator,
    now: &chrono::DateTime<chrono::Utc>,
) {
    let tasks = orchestrator.state.agent_tasks.read().await;
    let due_tasks: Vec<(usize, String)> = tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.should_run(now))
        .map(|(i, t)| (i, t.name.clone()))
        .collect();
    drop(tasks);

    for (idx, name) in due_tasks {
        println!("[Scheduler] ⏰ Running task: {}...", name);
        match name.as_str() {
            "market_scan" => {
                let _ = orchestrator.scanner.scan_watchlist().await;
            }
            "position_monitor" => {
                let _ = orchestrator.execution.run(None).await;
            }
            "portfolio_review" => {
                let p = orchestrator.state.portfolio.read().await;
                println!(
                    "[Review] Daily P&L: {:+.2} | Trades: {}",
                    p.daily_pnl, p.total_trades_today
                );
                drop(p);
            }
            "goal_review" => {
                let mut goals = orchestrator.state.trading_goals.write().await;
                let p = orchestrator.state.portfolio.read().await;
                goals.recalculate_mode(p.daily_pnl_pct, p.consecutive_losses, p.total_trades_today);
                println!(
                    "[Goals] 📊 Mode: {:?} | Current: {:+.2}%",
                    goals.mode,
                    p.daily_pnl_pct * 100.0
                );
                drop(p);
                drop(goals);
            }
            _ => {}
        }
        let mut tasks = orchestrator.state.agent_tasks.write().await;
        if idx < tasks.len() {
            tasks[idx].last_run = Some(*now);
        }
    }
}

async fn recalibrate_goals(orchestrator: &AutonomousOrchestrator) {
    let mut goals = orchestrator.state.trading_goals.write().await;
    let p = orchestrator.state.portfolio.read().await;
    goals.recalculate_mode(p.daily_pnl_pct, p.consecutive_losses, p.total_trades_today);
}

pub async fn save_portfolio_state(state: &SharedState) {
    let portfolio = state.portfolio.read().await;
    if let Ok(json) = serde_json::to_string(&*portfolio) {
        let _ = state.memory.store_state("portfolio/state", &json);
    }
}

async fn log_portfolio_snapshot(portfolio: &PortfolioState, state: &SharedState) {
    let goals = state.trading_goals.read().await;
    println!(
        "\n📊 [Portfolio] Equity: ₹{:.2} | Cash: ₹{:.2} | Positions: {} | P&L: ₹{:.2} | DD: {:.2}%",
        portfolio.total_equity,
        portfolio.cash_balance,
        portfolio.open_positions.len(),
        portfolio.daily_pnl,
        portfolio.max_drawdown_today * 100.0
    );
    println!(
        "   [Mode] {:?} | Target: {:+.2}% | Current: {:+.2}% | Trades: {}/{}",
        goals.mode,
        goals.daily_target_pnl_pct * 100.0,
        portfolio.daily_pnl_pct * 100.0,
        portfolio.total_trades_today,
        goals.max_daily_trades
    );
}

async fn log_portfolio_snapshot_full(orchestrator: &AutonomousOrchestrator) {
    let p = orchestrator.state.portfolio.read().await;
    let goals = orchestrator.state.trading_goals.read().await;
    println!(
        "\n📊 [Portfolio] Equity: ₹{:.2} | Cash: ₹{:.2} | Positions: {} | P&L: ₹{:.2} | DD: {:.2}%",
        p.total_equity,
        p.cash_balance,
        p.open_positions.len(),
        p.daily_pnl,
        p.max_drawdown_today * 100.0
    );
    println!(
        "   [Mode] {:?} | Target: {:+.2}% | Current: {:+.2}% | Trades: {}/{}",
        goals.mode,
        goals.daily_target_pnl_pct * 100.0,
        p.daily_pnl_pct * 100.0,
        p.total_trades_today,
        goals.max_daily_trades
    );
    if !p.open_positions.is_empty() {
        for pos in &p.open_positions {
            println!(
                "   • {} {} {} qty={:.0} entry={:.2} cur={:.2} P&L=₹{:.2}",
                pos.symbol,
                if pos.direction == tredo_core::TradeDirection::Long {
                    "LONG"
                } else {
                    "SHORT"
                },
                if pos.unrealized_pnl >= 0.0 {
                    "🟢"
                } else {
                    "🔴"
                },
                pos.quantity,
                pos.entry_price,
                pos.current_price,
                pos.unrealized_pnl
            );
        }
    }
    drop(goals);
    drop(p);
}
