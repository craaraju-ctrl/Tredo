// ═══════════════════════════════════════════════════════════════════════════════
// tredo-eventbus — Microservices Event Bus
//
// Provides an abstract EventBus trait with two implementations:
// 1. NatsEventBus — Production-grade NATS pub-sub for decoupled services
// 2. InMemoryEventBus — Tokio broadcast channels for testing/single-process mode
//
// Message subjects follow a hierarchical naming convention:
//   tredo.<domain>.<action>[.<detail>]
//
// Core subjects:
//   tredo.market.price.{symbol}      — Price updates (published by market-data)
//   tredo.market.ohlcv.{symbol}      — OHLCV bar data
//   tredo.signal.{symbol}            — Trade signals from pipeline
//   tredo.execution.order             — Order execution requests
//   tredo.execution.filled            — Order fill confirmations
//   tredo.portfolio.snapshot          — Portfolio state snapshots
//   tredo.health.{service}           — Health check events
//   tredo.system.control             — Start/stop/shutdown commands
//   tredo.cot.{symbol}               — Chain-of-thought entries
// ═══════════════════════════════════════════════════════════════════════════════

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::error::Error;
use tokio::sync::{broadcast, mpsc};

// ── Event Type Definitions ────────────────────────────────────────────────────

/// All event types that flow through the system.
/// Each variant is a serde-tagged enum for JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum TredoEvent {
    #[serde(rename = "market_price")]
    MarketPrice(MarketPriceEvent),

    #[serde(rename = "market_ohlcv")]
    MarketOhlcv(MarketOhlcvEvent),

    #[serde(rename = "signal")]
    Signal(SignalEvent),

    #[serde(rename = "execution_order")]
    ExecutionOrder(ExecutionOrderEvent),

    #[serde(rename = "execution_filled")]
    ExecutionFilled(ExecutionFilledEvent),

    #[serde(rename = "portfolio_snapshot")]
    PortfolioSnapshot(PortfolioSnapshotEvent),

    #[serde(rename = "health")]
    Health(HealthEvent),

    #[serde(rename = "system_control")]
    SystemControl(SystemControlEvent),

    #[serde(rename = "cot")]
    Cot(CotEvent),
}

// ── Market Data Events ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPriceEvent {
    pub symbol: String,
    pub price: f64,
    pub exchange: String,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketOhlcvBar {
    pub timestamp: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketOhlcvEvent {
    pub symbol: String,
    pub interval: String, // "1m", "5m", "1h", "1d", etc.
    pub bars: Vec<MarketOhlcvBar>,
    pub timestamp_micros: i64,
}

// ── Signal Events ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEvent {
    pub symbol: String,
    pub action: String, // "BUY", "SELL", "HOLD"
    pub entry_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub confidence: f64,
    pub reasoning: String,
    pub source: String, // "pipeline", "manual", "backtest"
    pub timestamp_micros: i64,
}

// ── Execution Events ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOrderEvent {
    pub symbol: String,
    pub direction: String,  // "BUY", "SELL"
    pub order_type: String, // "MARKET", "LIMIT", "STOP"
    pub quantity: f64,
    pub price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub client_order_id: Option<String>,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionFilledEvent {
    pub symbol: String,
    pub direction: String,
    pub quantity: f64,
    pub fill_price: f64,
    pub order_id: String,
    pub client_order_id: Option<String>,
    pub timestamp_micros: i64,
}

// ── Portfolio Events ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioSnapshotEvent {
    pub total_equity: f64,
    pub cash_balance: f64,
    pub daily_pnl: f64,
    pub open_positions_count: u32,
    pub total_trades_today: u32,
    pub winning_trades_today: u32,
    pub consecutive_losses: u32,
    pub timestamp_micros: i64,
}

// ── Health Events ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEvent {
    pub service: String,
    pub healthy: bool,
    pub latency_ms: Option<f64>,
    pub error_message: Option<String>,
    pub timestamp_micros: i64,
}

// ── System Control Events ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemControlEvent {
    pub command: String, // "START", "STOP", "RESTART", "SHUTDOWN", "RELOAD_CONFIG"
    pub target: Option<String>, // Optional: service to target (None = all)
    pub reason: String,
    pub timestamp_micros: i64,
}

// ── COT Events ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CotEvent {
    pub chain_id: u64,
    pub agent: String,
    pub action: String,
    pub reason: String,
    pub confidence: f64,
    pub symbol: Option<String>,
    pub timestamp_micros: i64,
}

// ── Event Bus Trait ───────────────────────────────────────────────────────────

/// Abstract event bus for pub-sub communication between microservices.
/// Implementations: NatsEventBus (production), InMemoryEventBus (testing).
///
/// Note: NOT `Clone` or `Send + Sync` on the trait — use `Arc<dyn EventBus>` for shared ownership.
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish an event to a subject.
    async fn publish(
        &self,
        subject: &str,
        event: &TredoEvent,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Subscribe to a subject pattern (supports NATS wildcards `*` and `>`).
    /// Returns a receiver that yields (subject, event) pairs.
    async fn subscribe(
        &self,
        pattern: &str,
    ) -> Result<Box<dyn EventStream>, Box<dyn Error + Send + Sync>>;

    /// Request-reply: publish a request and await a response.
    async fn request(
        &self,
        subject: &str,
        event: &TredoEvent,
        _timeout_secs: u64,
    ) -> Result<TredoEvent, Box<dyn Error + Send + Sync>>;

    /// Respond to a subject: handle incoming requests.
    async fn respond(
        &self,
        subject: &str,
    ) -> Result<Box<dyn EventStream>, Box<dyn Error + Send + Sync>>;

    /// Flush any pending messages.
    async fn flush(&self) -> Result<(), Box<dyn Error + Send + Sync>>;
}

/// A stream of (subject, event) pairs from a subscription.
#[async_trait]
pub trait EventStream: Send + Sync {
    async fn recv(&mut self) -> Option<(String, TredoEvent)>;
}

// ── NATS Implementation ───────────────────────────────────────────────────────

/// Production event bus using NATS pub-sub.
/// Services connect to a NATS server and communicate via subjects.
#[derive(Clone)]
pub struct NatsEventBus {
    client: async_nats::Client,
}

impl NatsEventBus {
    /// Connect to a NATS server. URL examples:
    /// - "demo.nats.io" (public demo)
    /// - "localhost:4222" (local server)
    /// - "nats://user:pass@host:4222" (authenticated)
    pub async fn connect(url: &str) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let client = async_nats::connect(url).await?;
        tracing::info!(url = %url, "Connected to NATS");
        Ok(Self { client })
    }

    /// Create a NATS event bus from an existing client (for testing).
    pub fn from_client(client: async_nats::Client) -> Self {
        Self { client }
    }

    /// Helper: serialize event to bytes.
    fn serialize(event: &TredoEvent) -> Result<bytes::Bytes, Box<dyn Error + Send + Sync>> {
        Ok(serde_json::to_vec(event)?.into())
    }

    /// Helper: deserialize event from bytes.
    fn deserialize(data: &[u8]) -> Result<TredoEvent, Box<dyn Error + Send + Sync>> {
        Ok(serde_json::from_slice(data)?)
    }
}

#[async_trait]
impl EventBus for NatsEventBus {
    async fn publish(
        &self,
        subject: &str,
        event: &TredoEvent,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let payload = Self::serialize(event)?;
        self.client.publish(subject.to_string(), payload).await?;
        Ok(())
    }

    async fn subscribe(
        &self,
        pattern: &str,
    ) -> Result<Box<dyn EventStream>, Box<dyn Error + Send + Sync>> {
        let subscriber = self.client.subscribe(pattern.to_string()).await?;
        Ok(Box::new(NatsStream { inner: subscriber }))
    }

    async fn request(
        &self,
        subject: &str,
        event: &TredoEvent,
        timeout_secs: u64,
    ) -> Result<TredoEvent, Box<dyn Error + Send + Sync>> {
        let payload = Self::serialize(event)?;
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            self.client.request(subject.to_string(), payload),
        )
        .await
        .map_err(|_| "NATS request timed out".to_string())??;

        Self::deserialize(&response.payload)
    }

    async fn respond(
        &self,
        subject: &str,
    ) -> Result<Box<dyn EventStream>, Box<dyn Error + Send + Sync>> {
        // Use queue group for load-balanced consumers
        let subscriber = self
            .client
            .queue_subscribe(subject.to_string(), "tredo-workers".to_string())
            .await?;
        Ok(Box::new(NatsStream { inner: subscriber }))
    }

    async fn flush(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.client.flush().await?;
        Ok(())
    }
}

/// NATS subscription stream wrapper.
pub struct NatsStream {
    inner: async_nats::Subscriber,
}

#[async_trait]
impl EventStream for NatsStream {
    async fn recv(&mut self) -> Option<(String, TredoEvent)> {
        let msg = self.inner.next().await?;
        match NatsEventBus::deserialize(&msg.payload) {
            Ok(event) => Some((msg.subject.to_string(), event)),
            Err(_) => {
                tracing::warn!(subject = %msg.subject, "Failed to deserialize message");
                None
            }
        }
    }
}

// ── In-Memory Implementation ──────────────────────────────────────────────────

/// In-memory event bus using tokio broadcast channels.
/// Used for testing and single-process mode (no NATS server needed).
/// Subscribers use client-side pattern filtering against a single broadcast channel.
#[derive(Clone)]
pub struct InMemoryEventBus {
    default_tx: broadcast::Sender<(String, TredoEvent)>,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { default_tx: tx }
    }
}

impl Default for InMemoryEventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a subject matches a NATS-style pattern.
/// Supports `*` (single segment) and `>` (multi-segment) wildcards.
fn subject_matches_pattern(subject: &str, pattern: &str) -> bool {
    let subject_parts: Vec<&str> = subject.split('.').collect();
    let pattern_parts: Vec<&str> = pattern.split('.').collect();

    let mut s_idx = 0;
    let mut p_idx = 0;

    while p_idx < pattern_parts.len() {
        let p = pattern_parts[p_idx];
        if p == ">" {
            // `>` matches zero or more segments — always a match
            return true;
        }
        if s_idx >= subject_parts.len() {
            return false; // Subject too short
        }
        let s = subject_parts[s_idx];
        if p != "*" && p != s {
            return false; // Mismatch
        }
        s_idx += 1;
        p_idx += 1;
    }

    // If pattern exhausted but subject has remaining segments, only match if last pattern was `>`
    s_idx >= subject_parts.len()
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(
        &self,
        subject: &str,
        event: &TredoEvent,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Publish to the single broadcast channel. Subscribers filter by pattern client-side.
        let _ = self.default_tx.send((subject.to_string(), event.clone()));
        Ok(())
    }

    async fn subscribe(
        &self,
        pattern: &str,
    ) -> Result<Box<dyn EventStream>, Box<dyn Error + Send + Sync>> {
        let pattern_owned = pattern.to_string();
        let (tx, rx) = mpsc::channel::<(String, TredoEvent)>(256);
        let mut default_rx = self.default_tx.subscribe();

        tokio::spawn(async move {
            loop {
                match default_rx.recv().await {
                    Ok((subject, event)) => {
                        if subject_matches_pattern(&subject, &pattern_owned)
                            && tx.send((subject, event)).await.is_err()
                        {
                            break; // Receiver dropped
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, pattern = %pattern_owned, "Broadcast receiver lagged");
                    }
                }
            }
        });

        Ok(Box::new(InMemoryStream { inner: rx }))
    }

    async fn request(
        &self,
        subject: &str,
        event: &TredoEvent,
        _timeout_secs: u64,
    ) -> Result<TredoEvent, Box<dyn Error + Send + Sync>> {
        // For in-memory, just publish and don't wait for response (simplified)
        self.publish(subject, event).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Err("InMemoryEventBus does not support request-reply — use NatsEventBus for that".into())
    }

    async fn respond(
        &self,
        subject: &str,
    ) -> Result<Box<dyn EventStream>, Box<dyn Error + Send + Sync>> {
        self.subscribe(subject).await
    }

    async fn flush(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
}

/// In-memory subscription stream wrapper.
pub struct InMemoryStream {
    inner: mpsc::Receiver<(String, TredoEvent)>,
}

#[async_trait]
impl EventStream for InMemoryStream {
    async fn recv(&mut self) -> Option<(String, TredoEvent)> {
        self.inner.recv().await
    }
}

// ── Event Bus Factory ─────────────────────────────────────────────────────────

/// Create an event bus based on the `EVENT_BUS_URL` environment variable.
/// - If set to a NATS URL (e.g., "localhost:4222"), creates NatsEventBus
/// - If unset or empty, creates InMemoryEventBus
pub async fn create_event_bus() -> Result<Box<dyn EventBus>, Box<dyn Error + Send + Sync>> {
    let url = std::env::var("EVENT_BUS_URL").unwrap_or_default();
    if url.is_empty() || url == "in-memory" {
        tracing::info!("Using in-memory event bus (single-process mode)");
        Ok(Box::new(InMemoryEventBus::new()))
    } else {
        let bus = NatsEventBus::connect(&url).await?;
        tracing::info!(url = %url, "NATS event bus connected");
        Ok(Box::new(bus))
    }
}

// ── Subject Helpers ───────────────────────────────────────────────────────────

/// Generate subject strings for common event types.
pub mod subjects {
    pub fn market_price(symbol: &str) -> String {
        format!("tredo.market.price.{}", symbol.to_lowercase())
    }
    pub fn market_ohlcv(symbol: &str) -> String {
        format!("tredo.market.ohlcv.{}", symbol.to_lowercase())
    }
    pub fn signal(symbol: &str) -> String {
        format!("tredo.signal.{}", symbol.to_lowercase())
    }
    pub fn execution_order() -> String {
        "tredo.execution.order".to_string()
    }
    pub fn execution_filled() -> String {
        "tredo.execution.filled".to_string()
    }
    pub fn portfolio_snapshot() -> String {
        "tredo.portfolio.snapshot".to_string()
    }
    pub fn health(service: &str) -> String {
        format!("tredo.health.{}", service.to_lowercase())
    }
    pub fn system_control() -> String {
        "tredo.system.control".to_string()
    }
    pub fn cot(symbol: &str) -> String {
        format!("tredo.cot.{}", symbol.to_lowercase())
    }

    // Wildcard patterns for subscriptions
    pub fn all_market_events() -> String {
        "tredo.market.*.*".to_string()
    }
    pub fn all_signal_events() -> String {
        "tredo.signal.*".to_string()
    }
    pub fn all_events() -> String {
        "tredo.>".to_string()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_publish_subscribe() {
        let bus = InMemoryEventBus::new();
        let mut sub = bus.subscribe("tredo.market.price.BTC").await.unwrap();

        let event = TredoEvent::MarketPrice(MarketPriceEvent {
            symbol: "BTC".to_string(),
            price: 50000.0,
            exchange: "binance".to_string(),
            timestamp_micros: chrono::Utc::now().timestamp_micros(),
        });

        bus.publish("tredo.market.price.BTC", &event).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.0, "tredo.market.price.BTC");
        match received.1 {
            TredoEvent::MarketPrice(price) => {
                assert_eq!(price.symbol, "BTC");
                assert!((price.price - 50000.0).abs() < 0.01);
            }
            _ => panic!("Expected MarketPrice event"),
        }
    }

    #[tokio::test]
    async fn test_in_memory_wildcard_subscription() {
        let bus = InMemoryEventBus::new();
        let mut sub = bus.subscribe("tredo.market.price.*").await.unwrap();

        bus.publish(
            "tredo.market.price.BTC",
            &TredoEvent::MarketPrice(MarketPriceEvent {
                symbol: "BTC".to_string(),
                price: 50000.0,
                exchange: "binance".to_string(),
                timestamp_micros: chrono::Utc::now().timestamp_micros(),
            }),
        )
        .await
        .unwrap();

        bus.publish(
            "tredo.market.price.ETH",
            &TredoEvent::MarketPrice(MarketPriceEvent {
                symbol: "ETH".to_string(),
                price: 3000.0,
                exchange: "binance".to_string(),
                timestamp_micros: chrono::Utc::now().timestamp_micros(),
            }),
        )
        .await
        .unwrap();

        let received1 = tokio::time::timeout(std::time::Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(received1.0.contains("BTC") || received1.0.contains("ETH"));
    }

    #[tokio::test]
    async fn test_signal_event_roundtrip() {
        let bus = InMemoryEventBus::new();
        let mut sub = bus.subscribe("tredo.signal.*").await.unwrap();

        let event = TredoEvent::Signal(SignalEvent {
            symbol: "SOL".to_string(),
            action: "BUY".to_string(),
            entry_price: 150.0,
            stop_loss: 145.0,
            take_profit: 165.0,
            confidence: 0.85,
            reasoning: "Bull flag breakout on volume".to_string(),
            source: "pipeline".to_string(),
            timestamp_micros: chrono::Utc::now().timestamp_micros(),
        });

        bus.publish("tredo.signal.SOL", &event).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();

        match received.1 {
            TredoEvent::Signal(sig) => {
                assert_eq!(sig.symbol, "SOL");
                assert_eq!(sig.action, "BUY");
            }
            _ => panic!("Expected Signal event"),
        }
    }

    #[tokio::test]
    async fn test_system_control_event() {
        let bus = InMemoryEventBus::new();
        let mut sub = bus.subscribe("tredo.system.control").await.unwrap();

        let event = TredoEvent::SystemControl(SystemControlEvent {
            command: "SHUTDOWN".to_string(),
            target: None,
            reason: "Scheduled maintenance".to_string(),
            timestamp_micros: chrono::Utc::now().timestamp_micros(),
        });

        bus.publish("tredo.system.control", &event).await.unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(1), sub.recv())
            .await
            .unwrap()
            .unwrap();

        match received.1 {
            TredoEvent::SystemControl(ctl) => {
                assert_eq!(ctl.command, "SHUTDOWN");
            }
            _ => panic!("Expected SystemControl event"),
        }
    }
}
