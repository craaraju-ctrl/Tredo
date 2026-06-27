//! Real REST API clients for live price data.
//! Supports Binance (crypto), Kraken (crypto fallback), CoinGecko (crypto fallback),
//! Yahoo Finance (stocks/indices).
//!
//! All endpoints are public and free — no API keys required.

use crate::data_feed::Bar;
use std::time::Duration;

// ── Symbol classification ───────────────────────────────────────────────────

/// Returns true if the symbol is a cryptocurrency supported by crypto exchanges.
pub fn is_crypto_symbol(symbol: &str) -> bool {
    tredo_core::is_crypto_symbol(symbol)
}

// ── Price fetching (single price tick) ──────────────────────────────────────

/// Fetch the latest price for a symbol using the best available source.
/// Crypto: Binance → CoinGecko → Kraken fallback.
/// Stocks/indices: Yahoo Finance.
pub async fn fetch_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    if is_crypto_symbol(symbol) {
        match tredo_core::fetch_binance_price(client, symbol).await {
            Ok(p) => Ok(p),
            Err(_) => match fetch_coingecko_price(client, symbol).await {
                Ok(p) => Ok(p),
                Err(_) => fetch_kraken_price(client, symbol).await,
            },
        }
    } else {
        fetch_yahoo_price(client, symbol).await
    }
}

/// Fetch latest price + 24h stats as a Bar-like structure.
/// Returns (price, high_24h, low_24h, volume_24h, change_pct).
async fn fetch_price_stats(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<(f64, f64, f64, f64, f64), Box<dyn std::error::Error + Send + Sync>> {
    if is_crypto_symbol(symbol) {
        fetch_binance_24hr_stats(client, symbol).await
    } else {
        fetch_yahoo_price_stats(client, symbol).await
    }
}

/// Fetch a full OHLCV bar (uses 24h stats + latest price).
pub async fn fetch_live_bar(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<Bar, Box<dyn std::error::Error + Send + Sync>> {
    let now = chrono::Utc::now();
    let price = fetch_price(client, symbol).await?;
    let (_, high, low, volume, _) = fetch_price_stats(client, symbol).await.unwrap_or((
        price,
        price * 1.02,
        price * 0.98,
        0.0,
        0.0,
    ));

    Ok(Bar {
        timestamp: now,
        open: price, // latest trade as open for live bar
        high,
        low,
        close: price,
        volume,
    })
}

// ── Binance ─────────────────────────────────────────────────────────────────

/// Fetch latest price from Binance.
pub async fn fetch_binance_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    tredo_core::fetch_binance_price(client, symbol).await
}

/// Fetch 24h ticker stats from Binance (for building a Bar).
async fn fetch_binance_24hr_stats(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<(f64, f64, f64, f64, f64), Box<dyn std::error::Error + Send + Sync>> {
    let ticker = tredo_core::fetch_ticker_24hr(client, symbol).await?;
    Ok((
        ticker.price,
        ticker.high_24h,
        ticker.low_24h,
        ticker.volume_24h,
        ticker.change_pct_24h,
    ))
}

/// Fetch klines (historical OHLCV bars) from Binance.
pub async fn fetch_binance_klines(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<Bar>, Box<dyn std::error::Error + Send + Sync>> {
    let ohlcv = tredo_core::fetch_klines(client, symbol, interval, limit).await?;
    Ok(ohlcv
        .into_iter()
        .map(|b| {
            let dt = chrono::DateTime::parse_from_rfc3339(&b.timestamp)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            Bar {
                timestamp: dt,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
            }
        })
        .collect())
}

// ── Kraken ──────────────────────────────────────────────────────────────────

/// Fetch price from Kraken.
pub async fn fetch_kraken_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let kraken_sym = match symbol {
        "BTC" => "XBTUSDT",
        "DOGE" => "XDGUSD",
        other => return Err(format!("Kraken: no mapping for {}", other).into()),
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

// ── CoinGecko ───────────────────────────────────────────────────────────────

/// Fetch price from CoinGecko (free, no auth, 10k+ coins).
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

fn symbol_to_coingecko_id(symbol: &str) -> String {
    match symbol {
        "BTC" => "bitcoin",
        "ETH" => "ethereum",
        "SOL" => "solana",
        "BNB" => "binancecoin",
        "XRP" => "ripple",
        "ADA" => "cardano",
        "DOGE" => "dogecoin",
        "AVAX" => "avalanche-2",
        "MATIC" => "matic-network",
        "LINK" => "chainlink",
        "DOT" => "polkadot",
        "ATOM" => "cosmos",
        "LTC" => "litecoin",
        "BCH" => "bitcoin-cash",
        "UNI" => "uniswap",
        "AAVE" => "aave",
        "NEAR" => "near",
        "ICP" => "internet-computer",
        "FIL" => "filecoin",
        "APT" => "aptos",
        "ARB" => "arbitrum",
        "OP" => "optimism",
        "SUI" => "sui",
        "INJ" => "injective-protocol",
        "TIA" => "celestia",
        "SEI" => "sei-network",
        "PEPE" => "pepe",
        "WIF" => "dogwifcoin",
        "SHIB" => "shiba-inu",
        "TON" => "the-open-network",
        "TRX" => "tron",
        "XLM" => "stellar",
        other => other,
    }
    .to_string()
}

// ── Yahoo Finance ───────────────────────────────────────────────────────────

/// Fetch latest price from Yahoo Finance.
pub async fn fetch_yahoo_price(
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

/// Fetch price statistics from Yahoo Finance.
async fn fetch_yahoo_price_stats(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<(f64, f64, f64, f64, f64), Box<dyn std::error::Error + Send + Sync>> {
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
        .unwrap_or(0.0);

    let quotes = &resp["chart"]["result"][0]["indicators"]["quote"][0];
    let high = quotes["high"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .filter_map(|v| v.as_f64())
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        })
        .unwrap_or(price);
    let low = quotes["low"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .filter_map(|v| v.as_f64())
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        })
        .unwrap_or(price);
    let volume: f64 = quotes["volume"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_f64().filter(|x| *x > 0.0))
                .sum()
        })
        .unwrap_or(0.0);
    let change_pct = resp["chart"]["result"][0]["meta"]["chartPreviousClose"]
        .as_f64()
        .map(|prev| {
            if prev > 0.0 {
                (price - prev) / prev * 100.0
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);

    Ok((price, high, low, volume, change_pct))
}

/// Fetch OHLCV bars from Yahoo.
pub async fn fetch_yahoo_ohlcv(
    client: &reqwest::Client,
    symbol: &str,
    limit: usize,
) -> Result<Vec<Bar>, Box<dyn std::error::Error + Send + Sync>> {
    let yahoo_symbol = match symbol {
        "NIFTY" => "^NSEI",
        "RELIANCE" => "RELIANCE.NS",
        other => other,
    };
    let range = if limit <= 100 { "1d" } else { "5d" };
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1m&range={}",
        yahoo_symbol, range
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
    let timestamps: Vec<i64> = result["timestamp"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
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

    let mut bars = Vec::with_capacity(n.min(limit));
    for i in 0..n.min(limit) {
        let dt =
            chrono::DateTime::from_timestamp(timestamps[i], 0).unwrap_or_else(chrono::Utc::now);
        bars.push(Bar {
            timestamp: dt,
            open: opens[i],
            high: highs[i],
            low: lows[i],
            close: closes[i],
            volume: volumes[i],
        });
    }
    Ok(bars)
}
