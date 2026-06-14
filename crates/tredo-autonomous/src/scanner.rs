use crate::state::SharedState;
use crate::types::MarketRegime;
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{
    calculate_confluence_score, calculate_pivot_points, Agent, AgentInput, AgentOutput, AgentTier,
    MarketContext,
};

/// WatchlistScannerAgent — scans for new trade opportunities and manages the dynamic watchlist.
/// Scans all watchlisted symbols, calculates confluence, and identifies high-conviction setups.
pub struct WatchlistScannerAgent {
    pub state: SharedState,
}

impl WatchlistScannerAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Scan all watched symbols and update their market regime + confluence scoring.
    pub async fn scan_watchlist(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        println!("[Scanner] 🔍 Scanning watchlist for high-conviction setups...");

        let watchlist = self.state.watchlist.read().await;
        let mut prices = Vec::with_capacity(watchlist.len());

        for sym in watchlist.iter() {
            let portfolio = self.state.portfolio.read().await;
            let from_open = portfolio
                .open_positions
                .iter()
                .find(|p| p.symbol == *sym)
                .map(|p| p.current_price);
            drop(portfolio);

            let price = if let Some(p) = from_open {
                p
            } else {
                let history = self.state.ohlcv_history.read().await;
                history
                    .get(sym)
                    .and_then(|h| h.last().map(|b| b.close))
                    .unwrap_or(0.0)
            };
            prices.push((sym.clone(), price));
        }
        drop(watchlist);

        let mut high_conviction = Vec::new();

        for (symbol, price) in &prices {
            if *price <= 0.0 {
                continue;
            }

            // Read real portfolio equity for accurate drawdown comparison
            let equity = {
                let portfolio = self.state.portfolio.read().await;
                portfolio.total_equity
            };
            let context = MarketContext {
                symbol: symbol.clone(),
                current_price: *price,
                high: price * 1.015,
                low: price * 0.985,
                previous_close: price * 0.998,
                timestamp: Utc::now(),
                daily_pnl: 0.0,
                equity,
                consecutive_losses: 0,
                is_red_folder_day: false,
                trend_direction: None,
            };

            let rules = self.state.rules.read().await;
            let pivots = calculate_pivot_points(
                context.high,
                context.low,
                context.previous_close,
                rules.pivot_method,
            );
            let confluence = calculate_confluence_score(&context, &pivots);
            drop(rules);

            let regime = self.state.market_regime.read().await;
            let regime_str = match *regime {
                Some(MarketRegime::TrendingBull) => "Bullish trend",
                Some(MarketRegime::TrendingBear) => "Bearish trend",
                Some(MarketRegime::Ranging) => "Ranging",
                Some(MarketRegime::Volatile) => "Volatile",
                _ => "Neutral",
            };
            drop(regime);

            println!(
                "[Scanner]   {} @ {:.2} | Confluence: {:.1}% | Regime: {} | Pivot: {:.2}",
                symbol,
                price,
                confluence * 100.0,
                regime_str,
                pivots.pivot
            );

            if confluence >= 0.65 {
                high_conviction.push(symbol.clone());
            }
        }

        if high_conviction.is_empty() {
            println!("[Scanner]   No high-conviction setups found.");
        } else {
            println!("[Scanner]   High-conviction: {:?}", high_conviction);
        }

        {
            let mut last_scan = self.state.last_watchlist_scan.write().await;
            *last_scan = Some(Utc::now());
        }

        Ok(high_conviction)
    }

    pub async fn add_to_watchlist(&self, symbol: &str) -> bool {
        let mut watchlist = self.state.watchlist.write().await;
        let upper = symbol.to_uppercase();
        if !watchlist.contains(&upper) {
            watchlist.push(upper.clone());
            println!("[Scanner] ➕ Added {} to watchlist", upper);
            true
        } else {
            false
        }
    }

    pub async fn remove_from_watchlist(&self, symbol: &str) -> bool {
        let mut watchlist = self.state.watchlist.write().await;
        let upper = symbol.to_uppercase();
        if let Some(pos) = watchlist.iter().position(|s| s == &upper) {
            watchlist.remove(pos);
            println!("[Scanner] ➖ Removed {} from watchlist", upper);
            true
        } else {
            false
        }
    }

    pub async fn get_watchlist(&self) -> Vec<String> {
        self.state.watchlist.read().await.clone()
    }
}

#[async_trait]
impl Agent for WatchlistScannerAgent {
    fn name(&self) -> &str {
        "WatchlistScannerAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Main
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let _ = self.scan_watchlist().await?;
        Ok(AgentOutput::Done)
    }
}
