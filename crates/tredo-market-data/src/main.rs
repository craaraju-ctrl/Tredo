// ═══════════════════════════════════════════════════════════════════════════════
// tredo-market-data — Standalone Market Data Service
//
// Fetches live prices and OHLCV data from Binance (crypto) and Yahoo Finance
// (stocks/indices), then broadcasts them via the NATS event bus.
//
// Architecture:
//   tredo-market-data ──(NATS publish)──→ Other services
//       ↑                            ↑
//   Binance API                  Yahoo Finance
//
// Subjects published:
//   tredo.market.price.{symbol}   — Latest price tick
//   tredo.market.ohlcv.{symbol}   — OHLCV bar data
//   tredo.health.market-data      — Health check status
//
// Listens on:
//   tredo.system.control          — Start/stop/shutdown commands
//
// This service runs as a standalone binary alongside the orchestrator.
// ═══════════════════════════════════════════════════════════════════════════════

use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::sleep;
use tracing::{info, warn};
use tredo_eventbus::{subjects, EventBus, TredoEvent};
use tredo_eventbus::{MarketOhlcvBar, MarketPriceEvent};

// ── Configuration ─────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct MarketDataConfig {
    symbols: Vec<String>,
    poll_interval_secs: u64,
    klines_limit: usize,
}

impl Default for MarketDataConfig {
    fn default() -> Self {
        Self {
            symbols: vec![
                "BTC".into(),
                "ETH".into(),
                "SOL".into(),
                "BNB".into(),
                "XRP".into(),
                "ADA".into(),
                "DOGE".into(),
                "AVAX".into(),
            ],
            poll_interval_secs: 5,
            klines_limit: 100,
        }
    }
}

// ── Application State ─────────────────────────────────────────────────────────

struct MarketDataService {
    bus: Arc<dyn EventBus>,
    config: MarketDataConfig,
    client: reqwest::Client,
    running: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
}

impl MarketDataService {
    fn new(bus: Arc<dyn EventBus>, config: MarketDataConfig) -> Self {
        Self {
            bus,
            config,
            client: reqwest::Client::new(),
            running: Arc::new(AtomicBool::new(true)),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Main loop: fetch and publish prices for all symbols.
    async fn run(self: Arc<Self>) {
        info!(
            symbols = self.config.symbols.len(),
            cadence_secs = self.config.poll_interval_secs,
            "Market data service started"
        );

        let mut tick_count = 0u64;

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => {
                    info!("Shutdown signal received, exiting");
                    break;
                }
                _ = sleep(Duration::from_secs(1)) => {
                    // Continue loop
                }
            }

            if !self.running.load(Ordering::Relaxed) {
                info!("Paused");
                continue;
            }

            let mut handles = Vec::new();

            for symbol in &self.config.symbols {
                let sym = symbol.clone();
                let client = self.client.clone();
                let bus = self.bus.clone();

                let handle = tokio::spawn(async move {
                    let is_crypto = is_crypto_symbol(&sym);

                    let price = match fetch_price(&client, &sym, is_crypto).await {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(symbol = %sym, error = %e, "Price fetch failed");
                            return;
                        }
                    };

                    if let Err(e) = bus
                        .publish(
                            &subjects::market_price(&sym),
                            &TredoEvent::MarketPrice(MarketPriceEvent {
                                symbol: sym.clone(),
                                price,
                                exchange: if is_crypto {
                                    "binance".into()
                                } else {
                                    "yahoo".into()
                                },
                                timestamp_micros: Utc::now().timestamp_micros(),
                            }),
                        )
                        .await
                    {
                        warn!(symbol = %sym, error = %e, "Publish error");
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }

            // Every 12 cycles (~1 minute), also fetch OHLCV data
            tick_count += 1;
            if tick_count.is_multiple_of(12) {
                let bus = self.bus.clone();
                let client = self.client.clone();
                let config_syms = self.config.symbols.clone();

                tokio::spawn(async move {
                    for symbol in &config_syms {
                        let is_crypto = is_crypto_symbol(symbol);
                        if let Ok(bars) = fetch_klines(&client, symbol, "1m", 100, is_crypto).await
                        {
                            let ohlcv_bars: Vec<MarketOhlcvBar> = bars
                                .iter()
                                .map(|b| MarketOhlcvBar {
                                    timestamp: b.timestamp.clone(),
                                    open: b.open,
                                    high: b.high,
                                    low: b.low,
                                    close: b.close,
                                    volume: b.volume,
                                })
                                .collect();

                            if !ohlcv_bars.is_empty() {
                                let _ = bus
                                    .publish(
                                        &subjects::market_ohlcv(symbol),
                                        &TredoEvent::MarketOhlcv(
                                            tredo_eventbus::MarketOhlcvEvent {
                                                symbol: symbol.clone(),
                                                interval: "1m".to_string(),
                                                bars: ohlcv_bars,
                                                timestamp_micros: Utc::now().timestamp_micros(),
                                            },
                                        ),
                                    )
                                    .await;
                            }
                        }
                    }
                });
            }

            // Publish health check
            let _ = self
                .bus
                .publish(
                    &subjects::health("market-data"),
                    &TredoEvent::Health(tredo_eventbus::HealthEvent {
                        service: "market-data".to_string(),
                        healthy: true,
                        latency_ms: None,
                        error_message: None,
                        timestamp_micros: Utc::now().timestamp_micros(),
                    }),
                )
                .await;

            sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
        }
    }
}

// ── Price Fetchers ────────────────────────────────────────────────────────────

fn is_crypto_symbol(symbol: &str) -> bool {
    tredo_core::is_crypto_symbol(symbol)
}

async fn fetch_price(
    client: &reqwest::Client,
    symbol: &str,
    is_crypto: bool,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    if is_crypto {
        tredo_core::fetch_binance_price(client, symbol).await
    } else {
        let yahoo_symbol = match symbol {
            "NIFTY" => "^NSEI",
            "RELIANCE" => "RELIANCE.NS",
            other => other,
        };
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{yahoo_symbol}?interval=1m&range=1d"
        );
        let resp: serde_json::Value = client
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
            )
            .timeout(Duration::from_secs(8))
            .send()
            .await?
            .json()
            .await?;
        let price = resp["chart"]["result"][0]["meta"]["regularMarketPrice"]
            .as_f64()
            .ok_or("regularMarketPrice field missing")?;
        Ok(price)
    }
}

#[derive(Debug, Clone)]
struct OhlcvBar {
    timestamp: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

async fn fetch_klines(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    limit: usize,
    is_crypto: bool,
) -> Result<Vec<OhlcvBar>, Box<dyn std::error::Error + Send + Sync>> {
    if !is_crypto {
        // Yahoo
        let yahoo_symbol = symbol;
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range=1d",
            yahoo_symbol
        );
        let resp: serde_json::Value = client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
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
            let dt = chrono::DateTime::from_timestamp(timestamps[i], 0).unwrap_or_else(Utc::now);
            bars.push(OhlcvBar {
                timestamp: dt.to_rfc3339(),
                open: opens[i],
                high: highs[i],
                low: lows[i],
                close: closes[i],
                volume: volumes[i],
            });
        }
        return Ok(bars);
    }

    let klines = tredo_core::fetch_klines(client, symbol, interval, limit).await?;
    Ok(klines
        .into_iter()
        .map(|b| OhlcvBar {
            timestamp: b.timestamp,
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
        })
        .collect())
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tredo_market_data=info".into()),
        )
        .init();

    println!("╔══════════════════════════════════════════════════════╗");
    println!(
        "║   tredo-market-data v{}                            ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("║   Standalone Market Data Service                   ║");
    println!("╚══════════════════════════════════════════════════════╝");

    // Create event bus (NATS if available, otherwise in-memory)
    let bus: Arc<dyn EventBus> = Arc::from(
        tredo_eventbus::create_event_bus()
            .await
            .expect("Failed to create event bus"),
    );

    // Load config
    let config = MarketDataConfig::default();

    // Subscribe to system control events
    let control_bus: Arc<dyn EventBus> = Arc::from(
        tredo_eventbus::create_event_bus()
            .await
            .expect("Failed to create control event bus"),
    );
    let mut control_sub = control_bus
        .subscribe(&subjects::system_control())
        .await
        .expect("Failed to subscribe to control events");

    let service = Arc::new(MarketDataService::new(bus, config));
    let service_clone = service.clone();
    let running = service_clone.running.clone();

    // Handle control events in background
    tokio::spawn(async move {
        while let Some((_subject, event)) = control_sub.recv().await {
            if let TredoEvent::SystemControl(ctl) = &event {
                info!(
                    command = %ctl.command,
                    reason = %ctl.reason,
                    "Received control command"
                );
                match ctl.command.as_str() {
                    "STOP" | "PAUSE" => {
                        running.store(false, Ordering::Relaxed);
                    }
                    "START" | "RESUME" => {
                        running.store(true, Ordering::Relaxed);
                    }
                    "SHUTDOWN" => {
                        info!("Shutting down...");
                        running.store(false, Ordering::Relaxed);
                        service_clone.shutdown.notify_waiters();
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        std::process::exit(0);
                    }
                    _ => {
                        warn!(command = %ctl.command, "Unknown command");
                    }
                }
            }
        }
    });

    // Run the main fetch loop
    service.run().await;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_crypto_symbol() {
        assert!(is_crypto_symbol("BTC"));
        assert!(is_crypto_symbol("ETH"));
        assert!(!is_crypto_symbol("NIFTY"));
        assert!(!is_crypto_symbol("AAPL"));
    }
}
