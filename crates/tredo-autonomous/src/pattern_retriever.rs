use crate::state::SharedState;
use crate::types::{PatternMatch, TradeSignal};
use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier};

pub struct PatternRetrieverAgent {
    pub state: SharedState,
}

impl PatternRetrieverAgent {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    /// Query SQLite history for past similar episodes and identify recurring patterns.
    /// This is the primary learning mechanism — real outcomes from past trades.
    pub async fn find_patterns(
        &self,
        symbol: &str,
    ) -> Result<Vec<PatternMatch>, Box<dyn Error + Send + Sync>> {
        println!("[PatternRetriever] Analyzing patterns for {}...", symbol);

        // Get current market context for similarity matching
        let (confluence, regime) = {
            let signals = self.state.last_signals.read().await;
            let conf = signals
                .iter()
                .rfind(|s| s.symbol == symbol)
                .map(|s| s.confluence_score)
                .unwrap_or(0.5);
            let r = self.state.market_regime.read().await;
            let regime_str = match *r {
                Some(crate::types::MarketRegime::TrendingBull) => "TrendingBull",
                Some(crate::types::MarketRegime::TrendingBear) => "TrendingBear",
                Some(crate::types::MarketRegime::Ranging) => "Ranging",
                Some(crate::types::MarketRegime::Volatile) => "Volatile",
                _ => "Unknown",
            }
            .to_string();
            (conf, regime_str)
        };

        // ── Primary: query SQLite for historically similar setups ────────────
        let similar = self
            .state
            .episode_store
            .find_similar_episodes(symbol, &regime, confluence, 10)
            .unwrap_or_default();

        let mut patterns: Vec<PatternMatch> = Vec::new();

        if !similar.is_empty() {
            let total = similar.len();
            let wins = similar.iter().filter(|e| e.outcome == "WIN").count();
            let avg_pnl: f64 = similar.iter().map(|e| e.pnl).sum::<f64>() / total as f64;
            let avg_regret: f64 =
                similar.iter().map(|e| e.regret_score).sum::<f64>() / total as f64;
            let win_rate = wins as f64 / total as f64;

            // Build lesson summary from most-recent high-regret episodes
            let lessons: Vec<String> = similar
                .iter()
                .filter(|e| e.regret_score >= 0.5)
                .take(3)
                .map(|e| e.lesson.clone())
                .collect();
            let lesson_str = if lessons.is_empty() {
                "No recent high-regret trades in similar conditions.".to_string()
            } else {
                format!("Past mistakes: {}", lessons.join("; "))
            };

            patterns.push(PatternMatch {
                pattern_key: format!("{}_sqlite_history_{}", symbol, regime),
                match_score: win_rate,
                historical_outcome: format!(
                    "{}/{} wins | avg P&L: ₹{:.0} | avg regret: {:.2} | {}",
                    wins, total, avg_pnl, avg_regret, lesson_str
                ),
                avg_return: avg_pnl,
                win_rate,
                total_occurrences: total,
            });

            println!(
                "[PatternRetriever] SQLite: {} similar episodes | WR: {:.1}% | avg P&L: ₹{:.0}",
                total,
                win_rate * 100.0,
                avg_pnl
            );
        } else {
            println!(
                "[PatternRetriever] No historical SQLite episodes for {} in {} regime",
                symbol, regime
            );
        }

        // ── Fallback: in-memory signal patterns (when history is sparse) ────
        let signals = self.state.last_signals.read().await;
        let relevant: Vec<&TradeSignal> = signals.iter().filter(|s| s.symbol == symbol).collect();

        if !relevant.is_empty() {
            // Pattern: direction bias
            let longs = relevant
                .iter()
                .filter(|s| s.direction == tredo_core::TradeDirection::Long)
                .count();
            let shorts = relevant
                .iter()
                .filter(|s| s.direction == tredo_core::TradeDirection::Short)
                .count();
            let total = longs + shorts;
            if total > 0 {
                let long_pct = longs as f64 / total as f64;
                patterns.push(PatternMatch {
                    pattern_key: format!("{}_direction_bias", symbol),
                    match_score: (long_pct - 0.5).abs() * 2.0,
                    historical_outcome: format!(
                        "LONG {:.0}% / SHORT {:.0}%",
                        long_pct * 100.0,
                        (1.0 - long_pct) * 100.0
                    ),
                    avg_return: 0.0,
                    win_rate: long_pct.max(1.0 - long_pct),
                    total_occurrences: total,
                });
            }
        }
        drop(signals);

        // Sort by match score descending
        patterns.sort_by(|a, b| {
            b.match_score
                .partial_cmp(&a.match_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        println!(
            "[PatternRetriever] Found {} patterns for {}",
            patterns.len(),
            symbol
        );
        for p in &patterns {
            println!(
                "[PatternRetriever]   {} | Score: {:.2} | WR: {:.1}% | {}",
                p.pattern_key,
                p.match_score,
                p.win_rate * 100.0,
                p.historical_outcome
            );
        }

        Ok(patterns)
    }

    /// Store a pattern observation into long-term memory
    pub async fn record_pattern(
        &self,
        symbol: &str,
        key: &str,
        data: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let memory_key = format!("patterns/{}/{}", symbol, key);
        let timestamped = format!("[{}] {}", Utc::now().to_rfc3339(), data);
        self.state
            .memory
            .store_decision(&memory_key, &timestamped)?;
        println!("[PatternRetriever] 💾 Stored pattern: {}", memory_key);
        Ok(())
    }
}

#[async_trait]
impl Agent for PatternRetrieverAgent {
    fn name(&self) -> &str {
        "PatternRetrieverAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        match input {
            Some(AgentInput::ConfluenceRequest { context }) => {
                let patterns = self.find_patterns(&context.symbol).await?;
                if !patterns.is_empty() {
                    let summary = patterns
                        .iter()
                        .map(|p| format!("{}: score={:.2}", p.pattern_key, p.match_score))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "[PatternRetriever] ✅ Pattern analysis complete: {}",
                        summary
                    );
                }
                Ok(AgentOutput::Done)
            }
            _ => {
                // Default: scan all symbols in watchlist for patterns
                let watchlist = self.state.watchlist.read().await;
                for sym in watchlist.iter() {
                    let _ = self.find_patterns(sym).await;
                }
                Ok(AgentOutput::Done)
            }
        }
    }
}
