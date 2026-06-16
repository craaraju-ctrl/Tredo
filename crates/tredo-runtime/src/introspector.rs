use crate::event_bus::{AgentEvent, EventBus};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tredo_autonomous::state::SharedState;
use tredo_autonomous::types::MarketRegime;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIntrospection {
    pub market_model_confidence: f64,
    pub data_quality: DataQuality,
    pub uncertainties: Vec<Uncertainty>,
    pub recent_accuracy: f64,
    pub recent_surprise_rate: f64,
    pub mode: AgentMode,
    pub missing_data: Vec<String>,
    pub total_observations: u64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DataQuality {
    Fresh,
    Acceptable,
    Stale,
    Unreliable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Uncertainty {
    pub topic: String,
    pub confidence: f64,
    pub reason: String,
    pub what_would_reduce_uncertainty: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AgentMode {
    ExploitConfident,
    ExploitCautious,
    Explore,
    Wait,
}

pub struct Introspector {
    state: SharedState,
    event_bus: EventBus,
    recent_outcomes: parking_lot::Mutex<VecDeque<bool>>,
    recent_forecast_errors: parking_lot::Mutex<VecDeque<f64>>,
    total_observations: AtomicU64,
}

impl Introspector {
    pub fn new(state: SharedState, event_bus: EventBus) -> Self {
        Self {
            state,
            event_bus,
            recent_outcomes: parking_lot::Mutex::new(VecDeque::with_capacity(50)),
            recent_forecast_errors: parking_lot::Mutex::new(VecDeque::with_capacity(50)),
            total_observations: AtomicU64::new(0),
        }
    }

    pub async fn run(self: Arc<Self>, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut rx = self.event_bus.subscribe();
        info!("Introspector started");
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                Ok(event) = rx.recv() => {
                    self.process_event(&event).await;
                }
            }
        }
        info!("Introspector stopped");
    }

    async fn process_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::PriceTick { .. } => {
                self.total_observations.fetch_add(1, Ordering::Relaxed);
            }
            AgentEvent::ForecastReceived { uncertainty, .. } => {
                if *uncertainty > 0.03 {
                    warn!("High forecast uncertainty: {:.3}", uncertainty);
                }
            }
            _ => {}
        }
    }

    pub async fn introspect(&self) -> AgentIntrospection {
        let data_quality = self.assess_data_quality().await;
        let model_conf = self.assess_model_confidence().await;
        let uncertainties = self.identify_uncertainties().await;
        let missing = self.find_missing_data().await;
        let mode = self.determine_mode(model_conf, &data_quality);
        let accuracy = self.compute_recent_accuracy();
        let surprise = self.compute_surprise_rate();

        AgentIntrospection {
            market_model_confidence: model_conf,
            data_quality,
            uncertainties,
            recent_accuracy: accuracy,
            recent_surprise_rate: surprise,
            mode,
            missing_data: missing,
            total_observations: self.total_observations.load(Ordering::Relaxed),
            last_updated: chrono::Utc::now(),
        }
    }

    async fn assess_data_quality(&self) -> DataQuality {
        let history = self.state.ohlcv_history.read().await;
        let mut staleness = Vec::new();
        for (_symbol, bars) in history.iter() {
            if let Some(last) = bars.last() {
                let age_min = chrono::DateTime::parse_from_rfc3339(&last.timestamp)
                    .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_minutes())
                    .unwrap_or(999);
                staleness.push(age_min);
            }
        }
        let max_age = staleness.iter().max().copied().unwrap_or(999);
        match max_age {
            0..=5 => DataQuality::Fresh,
            6..=15 => DataQuality::Acceptable,
            16..=60 => DataQuality::Stale,
            _ => DataQuality::Unreliable,
        }
    }

    async fn assess_model_confidence(&self) -> f64 {
        let accuracy = self.compute_recent_accuracy();
        let surprise = self.compute_surprise_rate();
        (accuracy * 0.6 + (1.0 - surprise) * 0.4).clamp(0.0, 1.0)
    }

    async fn identify_uncertainties(&self) -> Vec<Uncertainty> {
        let mut uncertainties = Vec::new();
        let regime = *self.state.market_regime.read().await;
        if let Some(r) = regime {
            if matches!(r, MarketRegime::Volatile) {
                uncertainties.push(Uncertainty {
                    topic: "regime".into(),
                    confidence: 0.3,
                    reason: "Market in high-volatility regime".into(),
                    what_would_reduce_uncertainty: "Wait for regime to stabilize (3+ bars)".into(),
                });
            }
        }
        let wrong_recent = self.recent_outcomes.lock().iter().rev().take(5).filter(|x| !**x).count();
        if wrong_recent >= 3 {
            uncertainties.push(Uncertainty {
                topic: "recent_decisions".into(),
                confidence: 0.4,
                reason: format!("{} of last 5 decisions were wrong", wrong_recent),
                what_would_reduce_uncertainty: "Reduce position size, tighten rules".into(),
            });
        }
        let avg_err = if !self.recent_forecast_errors.lock().is_empty() {
            let guard = self.recent_forecast_errors.lock();
            guard.iter().sum::<f64>() / guard.len() as f64
        } else {
            0.0
        };
        if avg_err > 0.02 {
            uncertainties.push(Uncertainty {
                topic: "forecast_accuracy".into(),
                confidence: 0.5,
                reason: format!("Recent forecast errors avg {:.1}%", avg_err * 100.0),
                what_would_reduce_uncertainty: "Smaller positions until forecast improves".into(),
            });
        }
        uncertainties
    }

    async fn find_missing_data(&self) -> Vec<String> {
        let mut missing = Vec::new();
        let history = self.state.ohlcv_history.read().await;
        let watchlist = self.state.watchlist.read().await;
        for sym in watchlist.iter() {
            if !history.contains_key(sym) || history.get(sym).map(|b| b.is_empty()).unwrap_or(true) {
                missing.push(format!("OHLCV history for {}", sym));
            }
        }
        missing
    }

    fn determine_mode(&self, model_conf: f64, data_q: &DataQuality) -> AgentMode {
        match (model_conf, data_q) {
            (c, DataQuality::Fresh) if c > 0.75 => AgentMode::ExploitConfident,
            (c, _) if c > 0.6 => AgentMode::ExploitCautious,
            (c, _) if c > 0.4 => AgentMode::Explore,
            _ => AgentMode::Wait,
        }
    }

    fn compute_recent_accuracy(&self) -> f64 {
        let guard = self.recent_outcomes.lock();
        if guard.is_empty() {
            return 0.5;
        }
        let wins = guard.iter().filter(|x| **x).count();
        wins as f64 / guard.len() as f64
    }

    fn compute_surprise_rate(&self) -> f64 {
        let guard = self.recent_forecast_errors.lock();
        if guard.is_empty() {
            return 0.0;
        }
        let big_surprises = guard.iter().filter(|e| **e > 0.05).count();
        big_surprises as f64 / guard.len() as f64
    }

    pub fn record_outcome(&self, was_correct: bool) {
        let mut guard = self.recent_outcomes.lock();
        guard.push_back(was_correct);
        if guard.len() > 50 {
            guard.pop_front();
        }
    }

    pub fn record_forecast_error(&self, error: f64) {
        let mut guard = self.recent_forecast_errors.lock();
        guard.push_back(error);
        if guard.len() > 50 {
            guard.pop_front();
        }
    }
}
