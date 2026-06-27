use crate::introspector::{AgentMode, Introspector};
use std::collections::HashMap;
use std::sync::Arc;
use tredo_autonomous::state::SharedState;
use tredo_core::TradeDirection;

pub struct ActiveLearner {
    state: SharedState,
    introspector: Option<Arc<Introspector>>,
    uncertainty_map: HashMap<(String, String), f64>,
    exploration_budget_pct: f64,
}

impl ActiveLearner {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            introspector: None,
            uncertainty_map: HashMap::new(),
            exploration_budget_pct: 0.02,
        }
    }

    pub fn with_introspector(mut self, introspector: Arc<Introspector>) -> Self {
        self.introspector = Some(introspector);
        self
    }

    pub async fn maybe_explore(&self, symbol: &str, _direction: TradeDirection) -> Option<f64> {
        if let Some(intro) = &self.introspector {
            let intro_state = intro.introspect().await;
            if !matches!(intro_state.mode, AgentMode::Explore) {
                return None;
            }
        }
        let unc = self.compute_symbol_uncertainty(symbol).await;
        if unc < 0.6 {
            return None;
        }
        let price = self.get_current_price(symbol).await;
        if price <= 0.0 {
            return None;
        }
        let equity = self.state.portfolio.read().await.total_equity;
        let max_probe = equity * self.exploration_budget_pct;
        if max_probe <= 0.0 {
            return None;
        }
        Some((max_probe / price) * 0.95)
    }

    pub fn record_probe_outcome(&mut self, symbol: &str, profitable: bool, surprise: f64) {
        let key = (symbol.to_string(), "exploration".to_string());
        let cur = self.uncertainty_map.get(&key).copied().unwrap_or(0.7);
        let new_unc = if profitable {
            cur * 0.8
        } else {
            (cur * 1.2).min(1.0)
        };
        let final_unc = if surprise > 0.05 {
            new_unc * 1.1
        } else {
            new_unc
        };
        self.uncertainty_map.insert(key, final_unc);
    }

    async fn compute_symbol_uncertainty(&self, _symbol: &str) -> f64 {
        0.9
    }

    async fn get_current_price(&self, symbol: &str) -> f64 {
        self.state
            .ohlcv_history
            .read()
            .await
            .get(symbol)
            .and_then(|b| b.last())
            .map(|b| b.close)
            .unwrap_or(0.0)
    }
}
