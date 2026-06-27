//! Data feed abstraction — unified interface for price data regardless of source.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single OHLCV bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A price tick (from WebSocket or simulated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tick {
    pub symbol: String,
    pub price: f64,
    pub volume: f64,
    pub timestamp: DateTime<Utc>,
}

/// Configuration for a data feed.
#[derive(Debug, Clone)]
pub struct FeedConfig {
    pub symbols: Vec<String>,
    pub interval_secs: u64,
    pub lookback_bars: usize,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

/// Unified interface for all data feeds.
#[async_trait]
pub trait DataFeed: Send + Sync {
    /// Get the next batch of bars (blocking if backtesting, non-blocking if live).
    async fn next_bars(&mut self) -> Result<Vec<Bar>, Box<dyn std::error::Error + Send + Sync>>;

    /// Get the current timestamp of the feed.
    fn current_time(&self) -> DateTime<Utc>;

    /// Whether this feed has more data (for backtest mode).
    fn has_next(&self) -> bool;

    /// Get feed name for logging.
    fn name(&self) -> &str;
}

/// Convert CSV record to Bar. Expected columns: timestamp,open,high,low,close,volume
impl Bar {
    pub fn from_csv_record(
        record: &csv::StringRecord,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let timestamp =
            chrono::DateTime::parse_from_rfc3339(record.get(0).ok_or("missing timestamp")?)?
                .with_timezone(&Utc);
        let open = record.get(1).ok_or("missing open")?.parse::<f64>()?;
        let high = record.get(2).ok_or("missing high")?.parse::<f64>()?;
        let low = record.get(3).ok_or("missing low")?.parse::<f64>()?;
        let close = record.get(4).ok_or("missing close")?.parse::<f64>()?;
        let volume = record.get(5).ok_or("missing volume")?.parse::<f64>()?;
        Ok(Self {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
        })
    }
}
