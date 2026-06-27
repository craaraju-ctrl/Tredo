//! Backtest data feed — reads historical data from CSV.

use crate::data_feed::{Bar, DataFeed, FeedConfig};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::Path;

/// Backtest feed that replays historical data from a CSV file.
pub struct BacktestFeed {
    bars: Vec<Bar>,
    cursor: usize,
    #[allow(dead_code)]
    config: FeedConfig,
    current_time: DateTime<Utc>,
}

impl BacktestFeed {
    /// Load bars from a CSV file.
    pub fn from_csv<P: AsRef<Path>>(
        path: P,
        config: FeedConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)?;

        let mut bars = Vec::new();
        for result in reader.records() {
            let record = result?;
            match Bar::from_csv_record(&record) {
                Ok(bar) => bars.push(bar),
                Err(e) => {
                    tracing::warn!("Skipping bad CSV row: {}", e);
                }
            }
        }

        bars.sort_by_key(|b| b.timestamp);
        tracing::info!("Loaded {} bars from CSV", bars.len());

        let current_time = bars.first().map(|b| b.timestamp).unwrap_or_else(Utc::now);

        Ok(Self {
            bars,
            cursor: 0,
            config,
            current_time,
        })
    }

    /// Create an empty backtest feed (for testing).
    pub fn empty(config: FeedConfig) -> Self {
        Self {
            bars: vec![],
            cursor: 0,
            config,
            current_time: Utc::now(),
        }
    }

    /// Get all bars loaded.
    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }

    /// Progress as a 0.0–1.0 fraction.
    pub fn progress(&self) -> f64 {
        if self.bars.is_empty() {
            return 1.0;
        }
        self.cursor as f64 / self.bars.len() as f64
    }
}

#[async_trait]
impl DataFeed for BacktestFeed {
    async fn next_bars(&mut self) -> Result<Vec<Bar>, Box<dyn std::error::Error + Send + Sync>> {
        if self.cursor >= self.bars.len() {
            return Ok(vec![]);
        }

        let bar = self.bars[self.cursor].clone();
        self.current_time = bar.timestamp;
        self.cursor += 1;

        Ok(vec![bar])
    }

    fn current_time(&self) -> DateTime<Utc> {
        self.current_time
    }

    fn has_next(&self) -> bool {
        self.cursor < self.bars.len()
    }

    fn name(&self) -> &str {
        "BacktestFeed (CSV)"
    }
}
