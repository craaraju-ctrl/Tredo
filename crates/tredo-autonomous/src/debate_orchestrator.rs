use crate::skills::ConfluenceScorer;

/// Outcome of a multi-agent debate.
#[derive(Debug, Clone)]
pub struct DebateOutcome {
    pub signal_approved: bool,
    pub final_direction: String,
    pub entry_level: f64,
    pub adjusted_stop_loss: f64,
    pub adjusted_take_profit: f64,
    pub calculated_confidence: f64,
    pub consensus_justification: String,
}

impl DebateOutcome {
    /// Create a HOLD outcome (no trade).
    pub fn hold(reason: &str) -> Self {
        Self {
            signal_approved: false,
            final_direction: "HOLD".to_string(),
            entry_level: 0.0,
            adjusted_stop_loss: 0.0,
            adjusted_take_profit: 0.0,
            calculated_confidence: 0.0,
            consensus_justification: reason.to_string(),
        }
    }
}

/// Orchestrates multi-agent debate sessions to reach consensus on trade decisions.
#[derive(Debug, Clone)]
pub struct DebateOrchestrator {
    pub regime_type: String,
    pub volatility_regime: String,
}

impl DebateOrchestrator {
    pub fn new(regime_type: &str, volatility_regime: &str) -> Self {
        Self {
            regime_type: regime_type.to_string(),
            volatility_regime: volatility_regime.to_string(),
        }
    }

    /// Run a debate session using the aggregated confluence score and historical memories.
    /// Returns a DebateOutcome with the consensus decision.
    pub async fn run_agent_debate(
        &self,
        symbol: &str,
        current_price: f64,
        aggregated: &ConfluenceScorer,
        _memories: &[String],
    ) -> DebateOutcome {
        // Simple deterministic debate logic based on aggregated signal
        if aggregated.conviction < 0.3 {
            return DebateOutcome::hold(&format!(
                "{}: Low conviction ({:.1}%) across skills. Holding.",
                symbol,
                aggregated.conviction * 100.0
            ));
        }

        if aggregated.is_bullish(None) && aggregated.conviction > 0.6 {
            let stop_loss = current_price * 0.98;
            let take_profit = current_price * 1.03;
            DebateOutcome {
                signal_approved: true,
                final_direction: "BUY".to_string(),
                entry_level: current_price,
                adjusted_stop_loss: stop_loss,
                adjusted_take_profit: take_profit,
                calculated_confidence: aggregated.conviction,
                consensus_justification: format!(
                    "Bullish consensus: net={:.2}, conviction={:.1}%. Entry={:.2}, SL={:.2}, TP={:.2}",
                    aggregated.net_score,
                    aggregated.conviction * 100.0,
                    current_price,
                    stop_loss,
                    take_profit
                ),
            }
        } else if aggregated.is_bearish(None) && aggregated.conviction > 0.6 {
            let stop_loss = current_price * 1.02;
            let take_profit = current_price * 0.97;
            DebateOutcome {
                signal_approved: true,
                final_direction: "SELL".to_string(),
                entry_level: current_price,
                adjusted_stop_loss: stop_loss,
                adjusted_take_profit: take_profit,
                calculated_confidence: aggregated.conviction,
                consensus_justification: format!(
                    "Bearish consensus: net={:.2}, conviction={:.1}%. Entry={:.2}, SL={:.2}, TP={:.2}",
                    aggregated.net_score,
                    aggregated.conviction * 100.0,
                    current_price,
                    stop_loss,
                    take_profit
                ),
            }
        } else {
            DebateOutcome::hold(&format!(
                "{}: Mixed signals (net={:.2}, conv={:.1}%). Holding.",
                symbol,
                aggregated.net_score,
                aggregated.conviction * 100.0
            ))
        }
    }
}
