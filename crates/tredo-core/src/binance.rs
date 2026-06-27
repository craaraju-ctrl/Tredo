//! Binance public REST API client — prices, 24h tickers, and klines.
//!
//! Handles symbol normalization, pair aliases, mirror fallbacks, and robust JSON parsing.

use crate::kronos_client::OhlcvBar;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::time::Duration;

const BASE_URLS: &[&str] = &[
    "https://api.binance.com",
    "https://api1.binance.com",
    "https://api2.binance.com",
    "https://api3.binance.com",
];

const EQUITY_SYMBOLS: &[&str] = &[
    "NIFTY",
    "SENSEX",
    "BANKNIFTY",
    "RELIANCE",
    "TCS",
    "INFY",
    "HDFC",
    "ICICIBANK",
    "AAPL",
    "MSFT",
    "GOOG",
    "AMZN",
    "TSLA",
    "NVDA",
    "META",
    "NFLX",
    "AMD",
    "INTC",
    "PYPL",
    "QCOM",
    "ADBE",
    "CRM",
    "CSCO",
    "PEP",
    "KO",
    "NKE",
    "DIS",
    "V",
    "MA",
    "JPM",
    "BAC",
    "WMT",
    "COST",
    "PG",
    "HD",
    "XOM",
    "CVX",
    "UNH",
    "LLY",
    "JNJ",
    "MRK",
    "PFE",
    "ABBV",
    "ABT",
    "MDT",
    "T",
    "VZ",
    "CMCSA",
    "C",
    "WFC",
    "MS",
    "GS",
    "BLK",
    "SCHW",
    "AMAT",
    "LRCX",
    "ASML",
    "TSM",
    "AVGO",
    "ORCL",
    "IBM",
    "ACN",
    "TXN",
    "MU",
    "NOW",
    "PANW",
    "FTNT",
    "CRWD",
    "DDOG",
    "NET",
    "OKTA",
    "SNOW",
    "U",
    "PLTR",
    "MSTR",
    "COIN",
    "HOOD",
    "BABA",
    "PDD",
    "JD",
    "BIDU",
    "NTES",
    "LI",
    "XPEV",
    "NIO",
    "TME",
    "F",
    "GM",
    "GE",
    "FSLR",
    "ENPH",
    "SEDG",
    "RUN",
    "SPWR",
    "CAT",
    "DE",
    "HON",
    "LMT",
    "GD",
    "NOC",
    "RTX",
    "BA",
    "UPS",
    "FDX",
    "SBUX",
    "MCD",
    "CMG",
    "YUM",
    "TGT",
    "TJX",
    "DG",
    "DLTR",
    "ROST",
    "ABNB",
];

const KNOWN_CRYPTO: &[&str] = &[
    "BTC", "ETH", "SOL", "BNB", "XRP", "ADA", "DOGE", "AVAX", "MATIC", "POL", "LINK", "DOT",
    "ATOM", "LTC", "BCH", "UNI", "AAVE", "NEAR", "ICP", "FIL", "APT", "ARB", "OP", "SUI", "INJ",
    "TIA", "SEI", "PEPE", "WIF", "SHIB", "TON", "TRX", "XLM", "BONK", "FLOKI", "RENDER", "FET",
    "RNDR", "HBAR", "VET", "ALGO", "FTM", "SAND", "MANA", "CRV", "MKR", "COMP", "SNX", "RUNE",
    "STX", "IMX", "GRT", "ENS", "LDO", "BLUR", "JUP", "PYTH", "WLD", "STRK", "ENA", "LUNA", "FTT",
    "KAVA", "ZIL", "AXS", "CHZ", "ENJ", "ONE", "HOT", "QTUM", "ONT", "BAT", "ZRX", "ZEC", "DASH",
    "XMR", "ETC", "EOS", "NEO", "IOTA", "XTZ", "KNC", "LRC", "SXP", "YFI", "BAL", "OXT", "SUSHI",
    "1INCH", "WOO", "JASMY", "GMT", "KAS", "ORDI", "AEVO", "ETHFI", "BOME", "MEW", "TURBO", "MEME",
    "NOT", "ONDO", "IO", "PENDLE", "JTO", "RAY", "FIDA", "OM", "ARK", "PHB", "ACH", "RSR", "CHR",
    "MINA", "DYDX", "GALA", "AR", "FLOW", "THETA", "EGLD", "CELO", "RLC", "GMX", "JOE", "ALPHA",
    "CVX", "FXS", "LQTY",
];

/// 24-hour ticker statistics for a symbol.
#[derive(Debug, Clone)]
pub struct Ticker24h {
    pub base_symbol: String,
    pub pair: String,
    pub price: f64,
    pub change_pct_24h: f64,
    pub high_24h: f64,
    pub low_24h: f64,
    pub volume_24h: f64,
    pub quote_volume_24h: f64,
    pub open_price: f64,
}

/// Normalize user-facing symbol to base asset (BTCUSDT → BTC).
pub fn normalize_base_symbol(symbol: &str) -> String {
    let upper = symbol.trim().to_uppercase();
    if upper.ends_with("USDT") {
        upper.trim_end_matches("USDT").to_string()
    } else if upper.ends_with("USD") {
        upper.trim_end_matches("USD").to_string()
    } else if upper.ends_with("BUSD") {
        upper.trim_end_matches("BUSD").to_string()
    } else {
        upper
    }
}

/// Candidate Binance spot pairs for a base symbol (primary + aliases).
pub fn pair_candidates(base: &str) -> Vec<String> {
    let base = normalize_base_symbol(base);
    let mut pairs = vec![format!("{base}USDT")];
    match base.as_str() {
        "MATIC" => pairs.push("POLUSDT".to_string()),
        "POL" => pairs.push("MATICUSDT".to_string()),
        "PEPE" => pairs.push("1000PEPEUSDT".to_string()),
        "SHIB" => pairs.push("1000SHIBUSDT".to_string()),
        "BONK" => pairs.push("1000BONKUSDT".to_string()),
        "FLOKI" => pairs.push("1000FLOKIUSDT".to_string()),
        "LUNC" => pairs.push("1000LUNCUSDT".to_string()),
        "XEC" => pairs.push("1000XECUSDT".to_string()),
        _ => {}
    }
    pairs.sort();
    pairs.dedup();
    pairs
}

pub fn to_binance_pair(symbol: &str) -> String {
    let norm = normalize_base_symbol(symbol);
    let std_pair = format!("{norm}USDT");
    let candidates = pair_candidates(symbol);
    if candidates.contains(&std_pair) {
        std_pair
    } else {
        candidates.into_iter().next().unwrap_or(std_pair)
    }
}

pub fn is_crypto_symbol(symbol: &str) -> bool {
    let upper = symbol.trim().to_uppercase();
    if upper.ends_with(".NS") || upper.ends_with(".BO") {
        return false;
    }
    if EQUITY_SYMBOLS.iter().any(|&eq| upper == eq) {
        return false;
    }
    let base = normalize_base_symbol(&upper);
    if upper.ends_with("USDT") || upper.ends_with("USD") {
        return true;
    }
    KNOWN_CRYPTO.contains(&base.as_str())
}

fn parse_f64(v: &Value) -> Option<f64> {
    v.as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| v.as_f64())
}

fn binance_error(v: &Value) -> Option<String> {
    let code = v.get("code")?.as_i64()?;
    if code >= 0 {
        return None;
    }
    let msg = v
        .get("msg")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown Binance error");
    Some(format!("Binance API {code}: {msg}"))
}

async fn get_text(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, String)],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut last_err: Option<String> = None;
    for base in BASE_URLS {
        let url = format!("{base}{path}");
        let mut req = client
            .get(&url)
            .header("User-Agent", "tredo/1.0")
            .timeout(Duration::from_secs(10));
        for (k, v) in query {
            req = req.query(&[(k, v.as_str())]);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    last_err = Some(format!("HTTP {status} from {base}{path}: {text}"));
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if let Some(err) = binance_error(&v) {
                        last_err = Some(err);
                        continue;
                    }
                }
                return Ok(text);
            }
            Err(e) => {
                last_err = Some(format!("{base}{path}: {e}"));
            }
        }
    }
    Err(last_err
        .unwrap_or_else(|| "all Binance mirrors failed".to_string())
        .into())
}

async fn get_json(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, String)],
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let text = get_text(client, path, query).await?;
    Ok(serde_json::from_str(&text)?)
}

fn map_tickers_to_requested(requested: &[&str], mut tickers: Vec<Ticker24h>) -> Vec<Ticker24h> {
    let mut by_base: std::collections::HashMap<String, Ticker24h> = tickers
        .drain(..)
        .map(|t| (t.base_symbol.clone(), t))
        .collect();
    let mut out = Vec::with_capacity(requested.len());
    for sym in requested {
        let base = normalize_base_symbol(sym);
        if let Some(t) = by_base.remove(&base) {
            out.push(t);
            continue;
        }
        // Alias mapping: MATIC request may resolve via POLUSDT ticker
        for candidate in pair_candidates(sym) {
            let alias_base = normalize_base_symbol(&candidate);
            if let Some(t) = by_base.remove(&alias_base) {
                let mut mapped = t;
                mapped.base_symbol = base.clone();
                out.push(mapped);
                break;
            }
        }
    }
    out
}

fn ticker_from_value(v: &Value) -> Option<Ticker24h> {
    let pair = v.get("symbol")?.as_str()?.to_string();
    let base = normalize_base_symbol(&pair);
    let price = parse_f64(v.get("lastPrice")?)?;
    Some(Ticker24h {
        base_symbol: base,
        pair,
        price,
        change_pct_24h: parse_f64(v.get("priceChangePercent")?).unwrap_or(0.0),
        high_24h: parse_f64(v.get("highPrice")?).unwrap_or(price),
        low_24h: parse_f64(v.get("lowPrice")?).unwrap_or(price),
        volume_24h: parse_f64(v.get("volume")?).unwrap_or(0.0),
        quote_volume_24h: parse_f64(v.get("quoteVolume")?).unwrap_or(0.0),
        open_price: parse_f64(v.get("openPrice")?).unwrap_or(price),
    })
}

/// Fetch latest price for a symbol (tries pair aliases).
pub async fn fetch_price(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    for pair in pair_candidates(symbol) {
        let query = vec![("symbol", pair)];
        match get_json(client, "/api/v3/ticker/price", &query).await {
            Ok(v) => {
                if let Some(price) = parse_f64(v.get("price").unwrap_or(&Value::Null)) {
                    return Ok(price);
                }
            }
            Err(_) => continue,
        }
    }
    Err(format!(
        "Binance price not found for {}",
        normalize_base_symbol(symbol)
    )
    .into())
}

/// Fetch 24h ticker for one symbol.
pub async fn fetch_ticker_24hr(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<Ticker24h, Box<dyn std::error::Error + Send + Sync>> {
    for pair in pair_candidates(symbol) {
        let query = vec![("symbol", pair)];
        match get_json(client, "/api/v3/ticker/24hr", &query).await {
            Ok(v) => {
                if let Some(ticker) = ticker_from_value(&v) {
                    return Ok(ticker);
                }
            }
            Err(_) => continue,
        }
    }
    Err(format!(
        "Binance 24h ticker not found for {}",
        normalize_base_symbol(symbol)
    )
    .into())
}

/// Batch-fetch 24h tickers for many symbols in one API call.
pub async fn fetch_tickers_24hr_batch(
    client: &reqwest::Client,
    symbols: &[&str],
) -> Result<Vec<Ticker24h>, Box<dyn std::error::Error + Send + Sync>> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }

    // Batch endpoint rejects invalid/alias pairs — use primary USDT pairs only.
    let pairs: Vec<String> = symbols.iter().map(|s| to_binance_pair(s)).collect();
    let symbols_json = serde_json::to_string(&pairs)?;
    let query = vec![("symbols", symbols_json)];

    match get_json(client, "/api/v3/ticker/24hr", &query).await {
        Ok(v) => {
            let arr = v.as_array().cloned().unwrap_or_default();
            let tickers: Vec<Ticker24h> = arr.iter().filter_map(ticker_from_value).collect();
            if !tickers.is_empty() {
                return Ok(map_tickers_to_requested(symbols, tickers));
            }
        }
        Err(e) => eprintln!("[Binance] batch 24hr failed: {e} — falling back to per-symbol"),
    }

    let mut out = Vec::new();
    for sym in symbols {
        if let Ok(t) = fetch_ticker_24hr(client, sym).await {
            out.push(t);
        }
    }
    Ok(out)
}

/// Convert ticker to the JSON shape expected by TUI / API consumers.
pub fn ticker_to_api_json(ticker: &Ticker24h, exchange: &str) -> Value {
    serde_json::json!({
        "price": ticker.price,
        "exchange": exchange,
        "binance": {
            "price": ticker.price,
            "change_pct_24h": ticker.change_pct_24h,
            "high_24h": ticker.high_24h,
            "low_24h": ticker.low_24h,
            "volume_24h": ticker.volume_24h,
            "quote_volume_24h": ticker.quote_volume_24h,
            "open_price": ticker.open_price,
            "pair": ticker.pair,
        }
    })
}

/// Fetch OHLCV klines from Binance.
pub async fn fetch_klines(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    limit: usize,
) -> Result<Vec<OhlcvBar>, Box<dyn std::error::Error + Send + Sync>> {
    let limit = limit.clamp(1, 1000);
    for pair in pair_candidates(symbol) {
        let query = vec![
            ("symbol", pair),
            ("interval", interval.to_string()),
            ("limit", limit.to_string()),
        ];
        match get_json(client, "/api/v3/klines", &query).await {
            Ok(v) => {
                if let Some(bars) = parse_klines_array(&v) {
                    if !bars.is_empty() {
                        return Ok(bars);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    Err(format!(
        "Binance klines not found for {} interval={interval}",
        normalize_base_symbol(symbol)
    )
    .into())
}

fn parse_klines_array(v: &Value) -> Option<Vec<OhlcvBar>> {
    let rows = v.as_array()?;
    let mut bars = Vec::with_capacity(rows.len());
    for row in rows {
        let kline = row.as_array()?;
        if kline.len() < 6 {
            continue;
        }
        let open_time = kline[0]
            .as_i64()
            .or_else(|| kline[0].as_f64().map(|f| f as i64))?;
        let open = parse_f64(&kline[1])?;
        let high = parse_f64(&kline[2])?;
        let low = parse_f64(&kline[3])?;
        let close = parse_f64(&kline[4])?;
        let volume = parse_f64(&kline[5]).unwrap_or(0.0);
        let dt = DateTime::from_timestamp_millis(open_time).unwrap_or_else(Utc::now);
        bars.push(OhlcvBar {
            timestamp: dt.to_rfc3339(),
            open,
            high,
            low,
            close,
            volume,
        });
    }
    Some(bars)
}

/// Legacy JSON shape for endpoints that expect raw Binance 24hr fields.
pub async fn fetch_ticker_24hr_raw(
    client: &reqwest::Client,
    symbol: &str,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    for pair in pair_candidates(symbol) {
        let query = vec![("symbol", pair)];
        if let Ok(v) = get_json(client, "/api/v3/ticker/24hr", &query).await {
            if v.get("lastPrice").is_some() {
                return Ok(v);
            }
        }
    }
    Ok(Value::Object(serde_json::Map::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_usdt() {
        assert_eq!(normalize_base_symbol("BTCUSDT"), "BTC");
        assert_eq!(normalize_base_symbol("btc"), "BTC");
    }

    #[test]
    fn pair_candidates_include_aliases() {
        let pepe = pair_candidates("PEPE");
        assert!(pepe.contains(&"PEPEUSDT".to_string()));
        assert!(pepe.contains(&"1000PEPEUSDT".to_string()));
    }

    #[test]
    fn is_crypto_rejects_equities() {
        assert!(!is_crypto_symbol("NIFTY"));
        assert!(!is_crypto_symbol("RELIANCE"));
        assert!(is_crypto_symbol("BTC"));
        assert!(is_crypto_symbol("ETHUSDT"));
    }

    #[tokio::test]
    async fn live_binance_batch_and_klines() {
        let client = reqwest::Client::new();
        let tickers = fetch_tickers_24hr_batch(&client, &["BTC", "ETH", "PEPE", "SHIB"])
            .await
            .expect("batch tickers");
        assert!(
            tickers.len() >= 4,
            "expected 4 tickers, got {}",
            tickers.len()
        );
        let klines = fetch_klines(&client, "BTC", "1m", 10)
            .await
            .expect("btc klines");
        assert!(!klines.is_empty());
    }
}
