use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use crate::episode_store::EpisodeStore;
use crate::meta_control::EvolvedMetaControl;
use crate::regime_classifier::MarketRegime;
use crate::state::SharedState;
use crate::tri_level_validator::TriLevelValidator;
use crate::weight_tuner::AttributionEngine;

#[derive(Debug)]
pub enum ProcessorError {
    DatabaseError(String),
    MissingMetadata(String),
}

impl std::fmt::Display for ProcessorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessorError::DatabaseError(msg) => {
                write!(f, "Database persistence failure: {}", msg)
            }
            ProcessorError::MissingMetadata(id) => {
                write!(f, "Missing pre-trade metadata for episode {}", id)
            }
        }
    }
}

impl std::error::Error for ProcessorError {}

/// The context containing the state of the skills and risk parameters
/// recorded at the exact moment the trade was opened.
#[derive(Debug, Clone)]
pub struct PreTradeSnapshot {
    pub episode_id: String,
    pub symbol: String,
    pub direction: String, // "BUY" or "SELL"
    pub entry_price: f64,
    pub rule_version: u32,
    pub active_weights: HashMap<String, f64>,
    pub skill_predictions: HashMap<String, f64>, // Pre-computed raw scores [-1.0, +1.0]
    /// Tri-level layer signals: rules, llm, kronos in [-1.0, +1.0]
    pub layer_predictions: HashMap<String, f64>,
}

/// Lightweight record for closed episodes (adapt/extend existing ClosedEpisode in episode_store).
#[derive(Debug, Clone)]
pub struct OutcomeProcessor {
    pub db: EpisodeStore,
    pub weight_tuner: AttributionEngine,
    pub meta_control: EvolvedMetaControl,
    // Volatile memory storage keeping track of pending trade metrics
    pending_snapshots: Arc<RwLock<HashMap<String, PreTradeSnapshot>>>,
}

impl OutcomeProcessor {
    pub fn new(
        db: EpisodeStore,
        weight_tuner: AttributionEngine,
        meta_control: EvolvedMetaControl,
    ) -> Self {
        Self {
            db,
            weight_tuner,
            meta_control,
            pending_snapshots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers the pre-trade context on order execution.
    /// Stores it in a lock-free thread-safe map until the trade closes.
    pub async fn register_pending_trade(&self, snapshot: PreTradeSnapshot) {
        self.pending_snapshots
            .write()
            .await
            .insert(snapshot.episode_id.clone(), snapshot);
    }

    /// Processes a closed trade, executes the learning backpropagation,
    /// triggers meta-rule evaluation, and commits the outcome to SQLite.
    pub async fn process_trade_close(
        &self,
        episode_id: &str,
        exit_price: f64,
        current_regime: MarketRegime,
        regime_stable_for_ticks: usize,
        current_config: &RiskGuardianConfig,
        state: Option<&SharedState>,
    ) -> Result<(HashMap<String, f64>, Option<RiskGuardianConfig>), ProcessorError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Pull pre-trade metadata out of active cache memory
        let pre_trade = {
            let mut map = self.pending_snapshots.write().await;
            map.remove(episode_id)
                .ok_or_else(|| ProcessorError::MissingMetadata(episode_id.to_string()))?
        };

        // 2. Calculate actual return characteristics of this trade
        let raw_return = if pre_trade.direction == "BUY" {
            (exit_price - pre_trade.entry_price) / pre_trade.entry_price
        } else {
            (pre_trade.entry_price - exit_price) / pre_trade.entry_price
        };

        // Account for default slippage and exchange fee penalties (0.15% estimate)
        let realized_pnl = raw_return - 0.0015;
        let was_correct = realized_pnl > 0.0;
        let regret_score = if was_correct { 0.0 } else { realized_pnl.abs() };

        let closed_record = ClosedEpisodeRecord {
            episode_id: episode_id.to_string(),
            symbol: pre_trade.symbol.clone(),
            direction: pre_trade.direction.clone(),
            entry_price: pre_trade.entry_price,
            exit_price,
            raw_pnl: realized_pnl * pre_trade.entry_price,
            pct_pnl: realized_pnl,
            entry_time: now - 300,
            exit_time: now,
            was_correct,
            regret_score,
            rule_version: pre_trade.rule_version,
        };

        // 3. Backpropagate learning rewards or penalties to optimize skill weights
        let weight_update = self.weight_tuner.tune_skill_weights(
            episode_id,
            realized_pnl,
            &pre_trade.direction,
            &pre_trade.skill_predictions,
            &pre_trade.active_weights,
            now,
        );

        // 4. Record per-timeframe prediction accuracy for learning
        //    (which TFs predicted correctly vs incorrectly)
        let outcome_label = if was_correct { "WIN" } else { "LOSS" };
        if let Ok(mtf_snapshots) = self.db.load_episode_mtf_snapshots(episode_id) {
            for (tf, dir, conf) in mtf_snapshots {
                let _ =
                    self.db
                        .insert_mtf_accuracy(&tf, &dir, outcome_label, conf, Some(episode_id));
            }
        }

        // 5. Tri-level attribution: upgrade rules/llm/kronos trust weights from live outcome
        if let Some(st) = state {
            if !pre_trade.layer_predictions.is_empty() {
                TriLevelValidator::attribute_and_upgrade(
                    st,
                    episode_id,
                    &pre_trade.direction,
                    realized_pnl,
                    &pre_trade.layer_predictions,
                )
                .await;
            }
        }

        // 6. Commit results to relational tables (SQLite)
        self.db
            .close_episode(&closed_record, &pre_trade.skill_predictions)
            .map_err(|e| ProcessorError::DatabaseError(format!("SQLite entry failed: {}", e)))?;

        // 5. Evaluate dynamic parameter risk scales based on regime stability
        let evolved_config = self.meta_control.evaluate_and_adapt(
            current_config,
            current_regime,
            regime_stable_for_ticks,
        );

        Ok((weight_update.updated_weights, evolved_config))
    }
}

use crate::risk_guardian::RiskGuardianConfig;

/// Intermediate record for closed episodes — used by OutcomeProcessor to pass
/// trade results to EpisodeStore for persistence.
#[derive(Debug, Clone)]
pub struct ClosedEpisodeRecord {
    pub episode_id: String,
    pub symbol: String,
    pub direction: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub raw_pnl: f64,
    pub pct_pnl: f64,
    pub entry_time: u64,
    pub exit_time: u64,
    pub was_correct: bool,
    pub regret_score: f64,
    pub rule_version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_store::EpisodeStore;
    use crate::meta_control::EvolvedMetaControl; // or the wrapper
    use crate::regime_classifier::MarketRegime;
    use crate::risk_guardian::RiskGuardianConfig;
    use crate::weight_tuner::AttributionEngine;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_outcome_processor_learning_loop() {
        // 1. Initialize our persistent database mock connection
        let db_store = EpisodeStore::open("file::memory:?cache=shared")
            .expect("Failed to create in-memory EpisodeStore");

        // 2. Set up our analytical and learning engines with the symmetric math
        let base_learning_rate = 0.10; // 10% adjustments
        let weight_tuner = AttributionEngine::new(base_learning_rate);

        let meta_control = EvolvedMetaControl::new(db_store.clone(), 0.05, 1);

        let processor = OutcomeProcessor::new(db_store.clone(), weight_tuner, meta_control);

        // 3. Register a mock trade execution with snapshot metrics
        let episode_id = "test_episode_001".to_string();

        let mut active_weights = HashMap::new();
        active_weights.insert("news_analyser".to_string(), 0.25);
        active_weights.insert("market_metrics_meter".to_string(), 0.25);

        let mut skill_predictions = HashMap::new();
        // Both skills predicted a highly confident bullish direction
        skill_predictions.insert("news_analyser".to_string(), 0.80);
        skill_predictions.insert("market_metrics_meter".to_string(), 0.60);

        let snapshot = PreTradeSnapshot {
            episode_id: episode_id.clone(),
            symbol: "BTCUSDT".to_string(),
            direction: "BUY".to_string(),
            entry_price: 65000.0,
            rule_version: 1,
            active_weights,
            skill_predictions,
            layer_predictions: HashMap::new(),
        };

        processor.register_pending_trade(snapshot).await;

        // 4. Simulate a market close event (Trade successfully hits the profit target)
        let exit_price = 68250.0; // Represents a +5% profit before fees
        let current_regime = MarketRegime::TrendingBull;
        let regime_stable_for_ticks = 10; // Regime is highly stable, permitting learning updates

        let current_config = RiskGuardianConfig::default_fallback();

        let (updated_weights, evolved_config) = processor
            .process_trade_close(
                &episode_id,
                exit_price,
                current_regime,
                regime_stable_for_ticks,
                &current_config,
                None,
            )
            .await
            .expect("Trade resolution and backpropagation must succeed");

        // 5. Verify mathematical invariants and database integrity
        let sum_weights: f64 = updated_weights.values().sum();
        assert!(
            (sum_weights - 1.0).abs() < 1e-9,
            "Weights must always remain normalized to 1.0"
        );

        let news_weight = updated_weights.get("news_analyser").copied().unwrap_or(0.0);
        let metrics_weight = updated_weights
            .get("market_metrics_meter")
            .copied()
            .unwrap_or(0.0);
        // news_analyser was more accurate (0.80 vs 0.60) → should get higher weight
        assert!(
            news_weight > metrics_weight,
            "The more accurate skill must be assigned higher weight"
        );

        assert!(
            evolved_config.is_none(),
            "MetaControl should not reduce risk during profitable periods"
        );
    }

    #[tokio::test]
    async fn test_regime_stability_and_meta_control_risk_squeezing() {
        let db_store = EpisodeStore::open("file::memory:?cache=shared")
            .expect("Failed to create in-memory EpisodeStore");

        let weight_tuner = AttributionEngine::new(0.10);
        let meta_control = EvolvedMetaControl::new(db_store.clone(), 0.05, 1);
        let processor = OutcomeProcessor::new(db_store.clone(), weight_tuner, meta_control);

        let episode_id = "test_episode_unstable_002".to_string();

        let mut active_weights = HashMap::new();
        active_weights.insert("news_analyser".to_string(), 0.50);
        active_weights.insert("market_metrics_meter".to_string(), 0.50);

        let mut skill_predictions = HashMap::new();
        skill_predictions.insert("news_analyser".to_string(), 0.80);
        skill_predictions.insert("market_metrics_meter".to_string(), 0.80);

        let snapshot = PreTradeSnapshot {
            episode_id: episode_id.clone(),
            symbol: "BTCUSDT".to_string(),
            direction: "BUY".to_string(),
            entry_price: 65000.0,
            rule_version: 1,
            active_weights,
            skill_predictions,
            layer_predictions: HashMap::new(),
        };

        processor.register_pending_trade(snapshot).await;

        let exit_price = 55000.0; // Significant drawdown
        let current_regime = MarketRegime::Volatile;
        let regime_stable_for_ticks = 2; // LESS than 5 ticks, indicating structural transition
        let current_config = RiskGuardianConfig::default_fallback();

        let (updated_weights, evolved_config) = processor
            .process_trade_close(
                &episode_id,
                exit_price,
                current_regime,
                regime_stable_for_ticks,
                &current_config,
                None,
            )
            .await
            .expect("Losing trade resolution must succeed");

        assert!(
            evolved_config.is_none(),
            "MetaControl must suppress rule modifications when the market regime is unstable to avoid mean reversion whip"
        );

        let sum_weights: f64 = updated_weights.values().sum();
        assert!(
            (sum_weights - 1.0).abs() < 1e-9,
            "Weights must always remain fully normalized to 1.0 even after losing periods"
        );
    }
}
