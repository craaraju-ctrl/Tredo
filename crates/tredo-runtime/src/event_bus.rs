use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    PriceTick {
        symbol: String,
        price: f64,
        volume: f64,
        timestamp: chrono::DateTime<chrono::Utc>,
        source: PriceSource,
    },
    NewsArrived {
        symbol: String,
        headline: String,
        sentiment: f64,
        source: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    CalendarEvent {
        title: String,
        impact: String,
        time_to_event_secs: i64,
    },
    PositionUpdate {
        symbol: String,
        current_pnl: f64,
        current_pnl_pct: f64,
        time_in_position_secs: i64,
    },
    ForecastReceived {
        symbol: String,
        median: f64,
        uncertainty: f64,
        implied_trend: f64,
    },
    InvestigationRequest {
        question: String,
        priority: u8,
    },
    GoalUpdate {
        description: String,
        target: String,
    },
    HypothesisUpdated {
        id: String,
        statement: String,
        status: HypothesisStatus,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PriceSource {
    WebSocket,
    Rest,
    Backtest,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HypothesisStatus {
    Confirmed,
    Rejected,
    Untestable,
}

pub struct EventBus {
    sender: broadcast::Sender<AgentEvent>,
    published_count: Arc<AtomicU64>,
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            published_count: self.published_count.clone(),
        }
    }
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            published_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn publish(&self, event: AgentEvent) {
        let count = self.published_count.fetch_add(1, Ordering::SeqCst);
        if count.is_multiple_of(100) {
            info!(target: "event_bus", "Published {} events so far", count);
        }
        match &event {
            AgentEvent::PriceTick { symbol, price, .. } => {
                debug!(target: "event_bus", "PriceTick {} @ {:.2}", symbol, price);
            }
            AgentEvent::NewsArrived { symbol, .. } => {
                info!(target: "event_bus", "NewsArrived for {}", symbol);
            }
            AgentEvent::Shutdown => {
                info!(target: "event_bus", "Shutdown event published");
            }
            _ => {}
        }
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.sender.subscribe()
    }

    /// Get a sender handle for components that need to publish events.
    pub fn sender(&self) -> broadcast::Sender<AgentEvent> {
        self.sender.clone()
    }

    pub fn published_count(&self) -> u64 {
        self.published_count.load(Ordering::SeqCst)
    }
}
