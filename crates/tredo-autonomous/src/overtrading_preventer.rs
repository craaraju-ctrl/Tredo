use crate::state::SharedState;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::RwLock;
use tredo_core::{Agent, AgentInput, AgentOutput, AgentTier, DisciplineCheck};

/// Maximum trades per symbol per day (research: 3 is optimal for precision)
const MAX_TRADES_PER_SYMBOL_PER_DAY: u32 = 3;

/// Maximum total trades per day across all symbols
const MAX_TOTAL_TRADES_PER_DAY: u32 = 8;

/// Tracks per-symbol trade history for frequency control
#[derive(Debug, Clone, Default)]
pub struct TradeFrequencyTracker {
    /// Symbol -> list of trade timestamps today
    pub trades_today: HashMap<String, Vec<DateTime<Utc>>>,
    /// Total trades across all symbols today
    pub total_today: u32,
}

impl TradeFrequencyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a trade for a symbol
    pub fn record_trade(&mut self, symbol: &str) {
        self.trades_today
            .entry(symbol.to_string())
            .or_default()
            .push(Utc::now());
        self.total_today += 1;
    }

    /// Check if a trade is allowed for this symbol
    /// Uses the configurable cooldown_secs from DisciplineRules (same as HardRulesGate).
    pub fn can_trade(&self, symbol: &str, cooldown_secs: u64) -> Result<(), String> {
        // Check total daily limit
        if self.total_today >= MAX_TOTAL_TRADES_PER_DAY {
            return Err(format!(
                "Total daily trade limit reached: {}/{}",
                self.total_today, MAX_TOTAL_TRADES_PER_DAY
            ));
        }

        // Check per-symbol daily limit
        let symbol_trades = self
            .trades_today
            .get(symbol)
            .map(|v| v.len() as u32)
            .unwrap_or(0);
        if symbol_trades >= MAX_TRADES_PER_SYMBOL_PER_DAY {
            return Err(format!(
                "Per-symbol daily limit reached for {}: {}/{}",
                symbol, symbol_trades, MAX_TRADES_PER_SYMBOL_PER_DAY
            ));
        }

        // Check cooldown using DisciplineRules.cooldown_secs (same value as HardRulesGate)
        if let Some(last_trades) = self.trades_today.get(symbol) {
            if let Some(last_time) = last_trades.last() {
                let elapsed = Utc::now() - *last_time;
                if elapsed.num_seconds() < cooldown_secs as i64 {
                    let remaining = cooldown_secs as i64 - elapsed.num_seconds();
                    return Err(format!(
                        "Cooldown active for {}: {}s remaining (min {}s between trades)",
                        symbol, remaining, cooldown_secs
                    ));
                }
            }
        }

        Ok(())
    }

    /// Reset daily counters (call at start of new trading day)
    pub fn reset_daily(&mut self) {
        self.trades_today.clear();
        self.total_today = 0;
    }
}

pub struct OvertradingPreventerAgent {
    pub state: SharedState,
    /// Shared frequency tracker — persists across agent invocations
    pub frequency_tracker: Arc<RwLock<TradeFrequencyTracker>>,
}

impl OvertradingPreventerAgent {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            frequency_tracker: Arc::new(RwLock::new(TradeFrequencyTracker::new())),
        }
    }
}

#[async_trait]
impl Agent for OvertradingPreventerAgent {
    fn name(&self) -> &str {
        "OvertradingPreventerAgent"
    }
    fn tier(&self) -> AgentTier {
        AgentTier::Sub
    }

    async fn run(
        &self,
        input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        // Extract symbol from input if available
        let symbol = match &input {
            Some(AgentInput::RiskRequest { context }) => Some(context.symbol.clone()),
            _ => None,
        };

        // Get portfolio state for basic checks
        let portfolio = self.state.portfolio.read().await;
        let total_trades_today = portfolio.total_trades_today;
        drop(portfolio);

        // Check frequency limits
        let tracker = self.frequency_tracker.read().await;
        let mut reasons = Vec::new();
        let mut passed = true;

        // Check total daily trades
        if total_trades_today >= MAX_TOTAL_TRADES_PER_DAY {
            reasons.push(format!(
                "Total daily trade limit reached: {}/{}",
                total_trades_today, MAX_TOTAL_TRADES_PER_DAY
            ));
            passed = false;
        }

        // Check per-symbol frequency if symbol is provided
        // Uses the same cooldown_secs from DisciplineRules as HardRulesGate for consistency
        let cooldown_secs = {
            let rules = self.state.rules.read().await;
            rules.cooldown_secs
        };
        if let Some(ref sym) = symbol {
            if let Err(e) = tracker.can_trade(sym, cooldown_secs) {
                reasons.push(e);
                passed = false;
            }
        }

        drop(tracker);

        println!(
            "[OvertradingPreventer] Trades today: {}/{} | Symbol: {} | Status: {}",
            total_trades_today,
            MAX_TOTAL_TRADES_PER_DAY,
            symbol.as_deref().unwrap_or("N/A"),
            if passed { "OK" } else { "BLOCKED" }
        );

        Ok(AgentOutput::RiskResult(DisciplineCheck {
            passed,
            reasons,
            confluence_score: None,
        }))
    }
}
