// ═══════════════════════════════════════════════════════════════════════════════
// tredo-metrics — Multi-Layer Monitoring & Alerting
//
// Standalone binary that collects trade outcomes and pipeline latency events,
// computes performance metrics (Sharpe, Sortino, Calmar, win rate, avg return),
// exposes a Prometheus /metrics endpoint, stores historical data in SQLite,
// and provides an alert rules engine with Slack/Email/Telegram notification.
//
// Architecture:
//   Orchestrator ──(POST /event {trade_outcome | latency})──→ Metrics
//   Orchestrator ──(POST /report/trigger)──→ Metrics generates weekly report
//   Prometheus    ──(GET /metrics)──→ Metrics (scrape target)
//   Metrics       ──(alert)──→ Slack / Email / Telegram
//
// This binary runs as a SEPARATE PROCESS alongside the orchestrator-watchdog-compliance stack.
// ═══════════════════════════════════════════════════════════════════════════════

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{error, info, warn};

// ── Constants ─────────────────────────────────────────────────────────────────

const HTTP_PORT: u16 = 9730;
const DB_PATH: &str = "~/.tredo/metrics.db";
const CONFIG_PATH: &str = "~/.tredo/metrics.toml";
const PROMETHEUS_NAMESPACE: &str = "tredo";

// ── Event Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum MetricsEvent {
    #[serde(rename = "trade_outcome")]
    TradeOutcome(TradeOutcomeEvent),
    #[serde(rename = "latency_sample")]
    LatencySample(LatencySampleEvent),
    #[serde(rename = "system_health")]
    SystemHealth(SystemHealthEvent),
    #[serde(rename = "pipeline_run")]
    PipelineRun(PipelineRunEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOutcomeEvent {
    pub symbol: String,
    pub direction: String, // "BUY" | "SELL"
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl: f64,        // Absolute P&L
    pub pnl_pct: f64,    // P&L as % of position
    pub confidence: f64, // 0-1
    pub win: bool,
    pub holding_time_secs: u64,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySampleEvent {
    pub component: String, // "pipeline", "hard_rules_gate", "debate", "execution", etc.
    pub duration_ms: f64,
    pub symbol: Option<String>,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemHealthEvent {
    pub service: String, // "kronos", "llm", "broker", "orchestrator", "watchdog"
    pub healthy: bool,
    pub latency_ms: Option<f64>,
    pub error_message: Option<String>,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRunEvent {
    pub symbol: String,
    pub action: String, // "BUY" | "SELL" | "HOLD"
    pub total_duration_ms: f64,
    pub layers: Vec<LayerTiming>,
    pub timestamp_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerTiming {
    pub name: String,
    pub duration_ms: f64,
    pub result: String,
}

// ── Performance Metrics ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceReport {
    pub generated_at: String,

    // Period
    pub period_start: String,
    pub period_end: String,

    // Trade statistics
    pub total_trades: u64,
    pub winning_trades: u64,
    pub losing_trades: u64,
    pub win_rate: f64,

    // P&L
    pub total_pnl: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub profit_factor: f64,
    pub largest_win: f64,
    pub largest_loss: f64,

    // Risk-adjusted
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub calmar_ratio: f64,
    pub max_drawdown_pct: f64,

    // Average metrics
    pub avg_confidence: f64,
    pub avg_holding_time_secs: f64,
    pub avg_return_per_trade_pct: f64,

    // Latency
    pub pipeline_p50_ms: f64,
    pub pipeline_p95_ms: f64,
    pub pipeline_p99_ms: f64,
    pub total_pipeline_runs: u64,

    // Health
    pub services_healthy: u32,
    pub services_total: u32,
    pub system_uptime_pct: f64,
}

// ── Alert Configuration ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// Slack webhook URL for alerts
    pub slack_webhook_url: Option<String>,
    /// Telegram bot token (overrides Notifier defaults)
    pub telegram_bot_token: Option<String>,
    /// Telegram chat ID
    pub telegram_chat_id: Option<String>,
    /// SMTP settings for email alerts
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: Option<String>,
    pub smtp_to: Option<String>,

    /// Alert rule thresholds
    pub max_consecutive_losses: u32,
    pub max_daily_drawdown_pct: f64,
    pub min_win_rate_pct: f64,
    pub max_latency_p99_ms: f64,
    pub consecutive_loss_window_secs: u64,

    /// Dedup: minimum seconds between identical alerts
    pub alert_dedup_secs: u64,
    /// Rate limit: max alerts per minute
    pub alerts_per_minute: u32,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            slack_webhook_url: None,
            telegram_bot_token: None,
            telegram_chat_id: None,
            smtp_host: None,
            smtp_port: Some(587),
            smtp_username: None,
            smtp_password: None,
            smtp_from: None,
            smtp_to: None,
            max_consecutive_losses: 5,
            max_daily_drawdown_pct: 8.0,
            min_win_rate_pct: 30.0,
            max_latency_p99_ms: 5000.0,
            consecutive_loss_window_secs: 86400,
            alert_dedup_secs: 300,
            alerts_per_minute: 5,
        }
    }
}

// ── Application State ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MetricsState {
    db: Arc<Mutex<Connection>>,
    alert_config: AlertConfig,
    /// Tracks last alert time per alert_key for dedup
    last_alert_time: Arc<Mutex<HashMap<String, i64>>>,
    /// Tracks alert timestamps for rate limiting
    alert_timestamps: Arc<Mutex<Vec<i64>>>,
    /// Startup time for uptime calculation
    startup_time: i64,
    client: reqwest::Client,
}

impl MetricsState {
    fn new(db: Connection, alert_config: AlertConfig) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            alert_config,
            last_alert_time: Arc::new(Mutex::new(HashMap::new())),
            alert_timestamps: Arc::new(Mutex::new(Vec::new())),
            startup_time: Utc::now().timestamp_micros(),
            client: reqwest::Client::new(),
        })
    }

    // ── Event Ingestion ────────────────────────────────────────────────────

    fn ingest_event(&self, event: &MetricsEvent) -> Result<(), rusqlite::Error> {
        match event {
            MetricsEvent::TradeOutcome(t) => self.store_trade_outcome(t),
            MetricsEvent::LatencySample(l) => self.store_latency_sample(l),
            MetricsEvent::SystemHealth(h) => self.store_system_health(h),
            MetricsEvent::PipelineRun(p) => self.store_pipeline_run(p),
        }
    }

    fn store_trade_outcome(&self, t: &TradeOutcomeEvent) -> Result<(), rusqlite::Error> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO trade_outcomes (
                timestamp_micros, symbol, direction, entry_price, exit_price,
                pnl, pnl_pct, confidence, win, holding_time_secs
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                t.timestamp_micros,
                t.symbol,
                t.direction,
                t.entry_price,
                t.exit_price,
                t.pnl,
                t.pnl_pct,
                t.confidence,
                t.win as i32,
                t.holding_time_secs
            ],
        )?;
        Ok(())
    }

    fn store_latency_sample(&self, l: &LatencySampleEvent) -> Result<(), rusqlite::Error> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO latency_samples (
                timestamp_micros, component, duration_ms, symbol
            ) VALUES (?1, ?2, ?3, ?4)",
            params![l.timestamp_micros, l.component, l.duration_ms, l.symbol],
        )?;
        Ok(())
    }

    fn store_system_health(&self, h: &SystemHealthEvent) -> Result<(), rusqlite::Error> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO system_health (
                timestamp_micros, service, healthy, latency_ms, error_message
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                h.timestamp_micros,
                h.service,
                h.healthy as i32,
                h.latency_ms,
                h.error_message
            ],
        )?;
        Ok(())
    }

    fn store_pipeline_run(&self, p: &PipelineRunEvent) -> Result<(), rusqlite::Error> {
        let db = self.db.lock().unwrap();
        let layers_json = serde_json::to_string(&p.layers).unwrap_or_default();
        db.execute(
            "INSERT INTO pipeline_runs (
                timestamp_micros, symbol, action, total_duration_ms, layers_json
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                p.timestamp_micros,
                p.symbol,
                p.action,
                p.total_duration_ms,
                layers_json
            ],
        )?;
        Ok(())
    }

    // ── Performance Metrics Calculation ────────────────────────────────────

    fn compute_performance(&self, since_micros: i64) -> PerformanceReport {
        let db = self.db.lock().unwrap();

        // Trade statistics
        let total_trades: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM trade_outcomes WHERE timestamp_micros >= ?1",
                params![since_micros],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let winning_trades: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM trade_outcomes WHERE timestamp_micros >= ?1 AND win = 1",
                params![since_micros],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let losing_trades = total_trades.saturating_sub(winning_trades);
        let win_rate = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64
        } else {
            0.0
        };

        // P&L statistics
        let (total_pnl, largest_win, largest_loss): (f64, f64, f64) = db
            .query_row(
                "SELECT COALESCE(SUM(pnl), 0), COALESCE(MAX(CASE WHEN win = 1 THEN pnl ELSE 0 END), 0),
                        COALESCE(MIN(CASE WHEN win = 0 THEN pnl ELSE 0 END), 0)
                 FROM trade_outcomes WHERE timestamp_micros >= ?1",
                params![since_micros],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap_or((0.0, 0.0, 0.0));

        let avg_win = if winning_trades > 0 {
            db.query_row(
                "SELECT COALESCE(AVG(pnl), 0) FROM trade_outcomes WHERE timestamp_micros >= ?1 AND win = 1",
                params![since_micros],
                |row| row.get(0),
            ).unwrap_or(0.0)
        } else {
            0.0
        };

        let avg_loss: f64 = if losing_trades > 0 {
            db.query_row(
                "SELECT COALESCE(AVG(pnl), 0) FROM trade_outcomes WHERE timestamp_micros >= ?1 AND win = 0",
                params![since_micros],
                |row| row.get(0),
            ).unwrap_or(0.0)
        } else {
            0.0
        };

        let profit_factor = if avg_loss.abs() > 0.001 {
            (avg_win * winning_trades as f64) / (avg_loss.abs() * losing_trades as f64)
        } else if total_pnl > 0.0 {
            999.0 // Infinite profit factor
        } else {
            0.0
        };

        // Risk-adjusted metrics — compute from per-trade returns
        let returns: Vec<f64> = {
            let mut stmt = db
                .prepare(
                    "SELECT pnl_pct FROM trade_outcomes WHERE timestamp_micros >= ?1 ORDER BY timestamp_micros"
                )
                .unwrap();
            let rows = stmt
                .query_map(params![since_micros], |row| row.get::<_, f64>(0))
                .unwrap();
            rows.filter_map(|r| r.ok()).collect()
        };

        let (sharpe_ratio, sortino_ratio) = compute_sharpe_sortino(&returns);
        let max_drawdown_pct = compute_max_drawdown(&returns);

        let calmar_ratio = if max_drawdown_pct > 0.001 {
            let annualized_return = if !returns.is_empty() {
                returns.iter().sum::<f64>() / returns.len() as f64 * 252.0 // Daily Sharpe approximation
            } else {
                0.0
            };
            annualized_return / max_drawdown_pct
        } else {
            0.0
        };

        // Averages
        let avg_confidence: f64 = db
            .query_row(
                "SELECT COALESCE(AVG(confidence), 0) FROM trade_outcomes WHERE timestamp_micros >= ?1",
                params![since_micros],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        let avg_holding_time: f64 = db
            .query_row(
                "SELECT COALESCE(AVG(holding_time_secs), 0) FROM trade_outcomes WHERE timestamp_micros >= ?1",
                params![since_micros],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        let avg_return_pct: f64 = db
            .query_row(
                "SELECT COALESCE(AVG(pnl_pct), 0) FROM trade_outcomes WHERE timestamp_micros >= ?1",
                params![since_micros],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        // Pipeline latency percentiles
        let latency_results: Vec<f64> = {
            let mut stmt = db
                .prepare(
                    "SELECT duration_ms FROM latency_samples
                     WHERE timestamp_micros >= ?1 AND component = 'pipeline'
                     ORDER BY duration_ms",
                )
                .unwrap();
            let rows = stmt
                .query_map(params![since_micros], |row| row.get::<_, f64>(0))
                .unwrap();
            rows.filter_map(|r| r.ok()).collect()
        };

        let lat_len = latency_results.len();
        let (p50, p95, p99) = if lat_len > 0 {
            let p50_val = percentile(&latency_results, 50.0);
            let p95_val = percentile(&latency_results, 95.0);
            let p99_val = percentile(&latency_results, 99.0);
            (p50_val, p95_val, p99_val)
        } else {
            (0.0, 0.0, 0.0)
        };

        let total_pipeline_runs: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM latency_samples WHERE timestamp_micros >= ?1 AND component = 'pipeline'",
                params![since_micros],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Health statistics
        let (healthy_count, total_health_checks): (u32, u32) = db
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN healthy = 1 THEN 1 ELSE 0 END), 0),
                        COUNT(*)
                 FROM system_health WHERE timestamp_micros >= ?1",
                params![since_micros],
                |row| {
                    Ok((
                        row.get::<_, u32>(0).unwrap_or(0),
                        row.get::<_, u32>(1).unwrap_or(0),
                    ))
                },
            )
            .unwrap_or((0, 0));

        let services_healthy = healthy_count.min(total_health_checks);
        let services_total = total_health_checks.max(1);
        let system_uptime_pct = if services_total > 0 {
            services_healthy as f64 / services_total as f64 * 100.0
        } else {
            100.0
        };

        let period_start = chrono::DateTime::from_timestamp_micros(since_micros)
            .unwrap_or_default()
            .to_rfc3339();

        PerformanceReport {
            generated_at: Utc::now().to_rfc3339(),
            period_start,
            period_end: Utc::now().to_rfc3339(),
            total_trades,
            winning_trades,
            losing_trades,
            win_rate,
            total_pnl,
            avg_win,
            avg_loss,
            profit_factor,
            largest_win,
            largest_loss,
            sharpe_ratio,
            sortino_ratio,
            calmar_ratio,
            max_drawdown_pct,
            avg_confidence,
            avg_holding_time_secs: avg_holding_time,
            avg_return_per_trade_pct: avg_return_pct,
            pipeline_p50_ms: p50,
            pipeline_p95_ms: p95,
            pipeline_p99_ms: p99,
            total_pipeline_runs,
            services_healthy,
            services_total,
            system_uptime_pct,
        }
    }

    // ── Prometheus Metrics ─────────────────────────────────────────────────

    fn format_prometheus(&self) -> String {
        let since = Utc::now().timestamp_micros() - 86_400_000_000; // Last 24 hours
        let perf = self.compute_performance(since);
        let db = self.db.lock().unwrap();
        let uptime_secs = (Utc::now().timestamp_micros() - self.startup_time) / 1_000_000;

        let mut lines = Vec::new();

        // Header
        lines.push(format!(
            "# HELP {0}_uptime_seconds System uptime",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {0}_uptime_seconds gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_uptime_seconds {}",
            PROMETHEUS_NAMESPACE, uptime_secs
        ));

        // Trade metrics
        lines.push(format!(
            "# HELP {}_trades_total Total trades executed",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_trades_total counter",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_trades_total {}",
            PROMETHEUS_NAMESPACE, perf.total_trades
        ));

        lines.push(format!(
            "# HELP {}_win_rate Win rate (0-1)",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!("# TYPE {}_win_rate gauge", PROMETHEUS_NAMESPACE));
        lines.push(format!(
            "{}_win_rate {:.4}",
            PROMETHEUS_NAMESPACE, perf.win_rate
        ));

        lines.push(format!(
            "# HELP {}_total_pnl Total P&L",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!("# TYPE {}_total_pnl gauge", PROMETHEUS_NAMESPACE));
        lines.push(format!(
            "{}_total_pnl {:.2}",
            PROMETHEUS_NAMESPACE, perf.total_pnl
        ));

        lines.push(format!(
            "# HELP {}_profit_factor Profit factor",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_profit_factor gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_profit_factor {:.4}",
            PROMETHEUS_NAMESPACE, perf.profit_factor
        ));

        // Risk-adjusted
        lines.push(format!(
            "# HELP {}_sharpe_ratio Annualized Sharpe ratio",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_sharpe_ratio gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_sharpe_ratio {:.4}",
            PROMETHEUS_NAMESPACE, perf.sharpe_ratio
        ));

        lines.push(format!(
            "# HELP {}_sortino_ratio Sortino ratio",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_sortino_ratio gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_sortino_ratio {:.4}",
            PROMETHEUS_NAMESPACE, perf.sortino_ratio
        ));

        lines.push(format!(
            "# HELP {}_calmar_ratio Calmar ratio",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_calmar_ratio gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_calmar_ratio {:.4}",
            PROMETHEUS_NAMESPACE, perf.calmar_ratio
        ));

        lines.push(format!(
            "# HELP {}_max_drawdown_pct Max drawdown %",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_max_drawdown_pct gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_max_drawdown_pct {:.4}",
            PROMETHEUS_NAMESPACE, perf.max_drawdown_pct
        ));

        // Latency
        lines.push(format!(
            "# HELP {}_pipeline_duration_ms Pipeline latency in ms",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_pipeline_duration_ms gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_pipeline_duration_ms{{quantile=\"0.50\"}} {:.1}",
            PROMETHEUS_NAMESPACE, perf.pipeline_p50_ms
        ));
        lines.push(format!(
            "{}_pipeline_duration_ms{{quantile=\"0.95\"}} {:.1}",
            PROMETHEUS_NAMESPACE, perf.pipeline_p95_ms
        ));
        lines.push(format!(
            "{}_pipeline_duration_ms{{quantile=\"0.99\"}} {:.1}",
            PROMETHEUS_NAMESPACE, perf.pipeline_p99_ms
        ));

        // Health
        lines.push(format!(
            "# HELP {}_services_healthy Number of healthy services",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_services_healthy gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_services_healthy {}",
            PROMETHEUS_NAMESPACE, perf.services_healthy
        ));

        lines.push(format!(
            "# HELP {}_system_uptime_pct System uptime %",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "# TYPE {}_system_uptime_pct gauge",
            PROMETHEUS_NAMESPACE
        ));
        lines.push(format!(
            "{}_system_uptime_pct {:.2}",
            PROMETHEUS_NAMESPACE, perf.system_uptime_pct
        ));

        // Per-component latency from recent samples
        let components = [
            "pipeline",
            "hard_rules_gate",
            "debate",
            "execution",
            "market_intel",
        ];
        for comp in &components {
            let avg_lat: f64 = db
                .query_row(
                    "SELECT COALESCE(AVG(duration_ms), 0) FROM latency_samples
                     WHERE component = ?1 AND timestamp_micros >= ?2",
                    params![comp, since],
                    |row| row.get(0),
                )
                .unwrap_or(0.0);
            if avg_lat > 0.0 {
                lines.push(format!(
                    "{}_component_latency_ms{{component=\"{}\"}} {:.1}",
                    PROMETHEUS_NAMESPACE, comp, avg_lat
                ));
            }
        }

        lines.join("\n")
    }

    // ── Alert Engine ───────────────────────────────────────────────────────

    fn check_alerts(&self, event: &MetricsEvent) -> Result<(), rusqlite::Error> {
        let config = &self.alert_config;
        if !self.rate_limit_allowed() {
            return Ok(()); // Rate limited — skip
        }

        match event {
            MetricsEvent::TradeOutcome(t) => {
                // Check consecutive losses
                if !t.win {
                    let recent_losses: u32 = {
                        let db = self.db.lock().unwrap();
                        let since = Utc::now().timestamp_micros()
                            - config.consecutive_loss_window_secs as i64 * 1_000_000;
                        db.query_row(
                            "SELECT COUNT(*) FROM trade_outcomes
                             WHERE timestamp_micros >= ?1 AND win = 0",
                            params![since],
                            |row| row.get(0),
                        )
                        .unwrap_or(0)
                    };

                    if recent_losses >= config.max_consecutive_losses {
                        let alert_key = format!("consecutive_losses_{}", recent_losses);
                        if !self.is_duplicate(&alert_key) {
                            let msg = format!(
                                "⚠️ {} consecutive losses detected (threshold: {}). Last: {} {} @ {:.2} (P&L: ₹{:.2})",
                                recent_losses, config.max_consecutive_losses,
                                t.symbol, t.direction, t.entry_price, t.pnl
                            );
                            self.send_alert("Consecutive Loss Alert", &msg);
                        }
                    }
                }

                // Check daily drawdown
                let today_start = Utc::now()
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp_micros();
                let today_pnl: f64 = {
                    let db = self.db.lock().unwrap();
                    db.query_row(
                        "SELECT COALESCE(SUM(pnl), 0) FROM trade_outcomes WHERE timestamp_micros >= ?1",
                        params![today_start],
                        |row| row.get(0),
                    ).unwrap_or(0.0)
                };
                if today_pnl < 0.0 {
                    // We need initial equity to compute % — use a default estimation
                    // The metric service doesn't have direct access to portfolio equity,
                    // so we'll check the rate of loss instead
                    if today_pnl.abs() > 10000.0 {
                        let alert_key = "large_daily_loss";
                        if !self.is_duplicate(alert_key) {
                            let msg = format!(
                                "📉 Large daily loss: ₹{:.2} from last trade {} {} @ {:.2}",
                                today_pnl, t.symbol, t.direction, t.entry_price
                            );
                            self.send_alert("Large Daily Loss", &msg);
                        }
                    }
                }
            }
            MetricsEvent::LatencySample(l) => {
                // Check P99 latency threshold
                if l.duration_ms > config.max_latency_p99_ms {
                    let alert_key = format!(
                        "high_latency_{}_{}",
                        l.component,
                        l.duration_ms as u64 / 1000
                    );
                    if !self.is_duplicate(&alert_key) {
                        let msg = format!(
                            "🐌 High latency: {} took {:.0}ms (threshold: {:.0}ms){}",
                            l.component,
                            l.duration_ms,
                            config.max_latency_p99_ms,
                            l.symbol
                                .as_ref()
                                .map(|s| format!(" for {}", s))
                                .unwrap_or_default()
                        );
                        self.send_alert("Latency Alert", &msg);
                    }
                }
            }
            MetricsEvent::SystemHealth(h) => {
                if !h.healthy {
                    let alert_key = format!("service_down_{}", h.service);
                    if !self.is_duplicate(&alert_key) {
                        let msg = format!(
                            "🔴 Service DOWN: {}{}",
                            h.service,
                            h.error_message
                                .as_ref()
                                .map(|e| format!(" — {}", e))
                                .unwrap_or_default()
                        );
                        self.send_alert("Service Down", &msg);
                    }
                }
            }
            MetricsEvent::PipelineRun(_p) => {
                // Check if pipeline keeps producing HOLD (stuck)
                let recent_holds: u64 = {
                    let db = self.db.lock().unwrap();
                    let thirty_min_ago = Utc::now().timestamp_micros() - 30 * 60 * 1_000_000;
                    db.query_row(
                        "SELECT COUNT(*) FROM pipeline_runs
                         WHERE timestamp_micros >= ?1 AND action = 'HOLD'",
                        params![thirty_min_ago],
                        |row| row.get(0),
                    )
                    .unwrap_or(0)
                };

                if recent_holds >= 10 {
                    let alert_key = "excessive_holds";
                    if !self.is_duplicate(alert_key) {
                        let msg = format!(
                            "⏸️ {} HOLD decisions in last 30 minutes. Pipeline may be stuck.",
                            recent_holds
                        );
                        self.send_alert("Pipeline Stuck", &msg);
                    }
                }
            }
        }
        Ok(())
    }

    fn rate_limit_allowed(&self) -> bool {
        let mut timestamps = self.alert_timestamps.lock().unwrap();
        let now = Utc::now().timestamp_micros();
        let one_minute_ago = now - 60 * 1_000_000;

        // Remove old timestamps
        timestamps.retain(|&t| t > one_minute_ago);

        if timestamps.len() >= self.alert_config.alerts_per_minute as usize {
            false
        } else {
            timestamps.push(now);
            true
        }
    }

    fn is_duplicate(&self, key: &str) -> bool {
        let mut last_times = self.last_alert_time.lock().unwrap();
        let now = Utc::now().timestamp_micros();
        let dedup_micros = self.alert_config.alert_dedup_secs as i64 * 1_000_000;

        if let Some(&last) = last_times.get(key) {
            if now - last < dedup_micros {
                return true; // Duplicate — skipped
            }
        }
        last_times.insert(key.to_string(), now);
        false
    }

    fn send_alert(&self, title: &str, message: &str) {
        let full_msg = format!("[tredo-metrics] {} — {}", title, message);

        // Log locally
        warn!("[Alert] {}", full_msg);

        // Send to Slack
        if let Some(ref url) = self.alert_config.slack_webhook_url {
            let body = serde_json::json!({
                "text": full_msg,
                "username": "tredo-metrics",
                "icon_emoji": ":chart_with_upwards_trend:"
            });
            let client = self.client.clone();
            let url = url.clone();
            tokio::spawn(async move {
                if let Err(e) = client.post(&url).json(&body).send().await {
                    error!("[Alert] Slack delivery failed: {}", e);
                }
            });
        }

        // Send to Telegram
        if let (Some(ref token), Some(ref chat_id)) = (
            &self.alert_config.telegram_bot_token,
            &self.alert_config.telegram_chat_id,
        ) {
            let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": full_msg,
                "parse_mode": "Markdown"
            });
            let client = self.client.clone();
            tokio::spawn(async move {
                if let Err(e) = client.post(&url).json(&body).send().await {
                    error!("[Alert] Telegram delivery failed: {}", e);
                }
            });
        }

        // Send to Email via SMTP (simplified — uses external SMTP service)
        if let (Some(ref host), Some(ref _to)) =
            (&self.alert_config.smtp_host, &self.alert_config.smtp_to)
        {
            let port = self.alert_config.smtp_port.unwrap_or(587);
            let _from = self
                .alert_config
                .smtp_from
                .clone()
                .unwrap_or_else(|| "tredo@metrics.local".to_string());
            let body_text = format!("Subject: [tredo] {}\n\n{}", title, full_msg);

            // Attempt to send via SMTP using reqwest mailgun or similar
            let client = self.client.clone();
            let url = format!("https://{}:{}/send", host, port);
            tokio::spawn(async move {
                if let Err(e) = client
                    .post(&url)
                    .header("Content-Type", "text/plain")
                    .body(body_text)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                {
                    error!("[Alert] Email delivery failed: {}", e);
                }
            });
        }
    }

    // ── Weekly Report ──────────────────────────────────────────────────────

    fn generate_weekly_report(&self) -> PerformanceReport {
        let week_ago = Utc::now().timestamp_micros() - 7 * 86_400 * 1_000_000;
        self.compute_performance(week_ago)
    }

    fn format_report_text(&self, report: &PerformanceReport) -> String {
        format!(
            r#"📊 tredo Weekly Performance Report
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Period: {} — {}

📈 TRADE STATISTICS
  Total Trades:     {}
  Win Rate:         {:.1}%
  Profit Factor:    {:.2}

💰 P&L ANALYSIS
  Total P&L:        ₹{:.2}
  Avg Win:          ₹{:.2}
  Avg Loss:         ₹{:.2}
  Largest Win:      ₹{:.2}
  Largest Loss:     ₹{:.2}
  Avg Return/Trade: {:.2}%

📊 RISK-ADJUSTED METRICS
  Sharpe Ratio:     {:.2}
  Sortino Ratio:    {:.2}
  Calmar Ratio:     {:.2}
  Max Drawdown:     {:.2}%

⏱ PIPELINE PERFORMANCE
  Total Runs:       {}
  P50 Latency:      {:.0}ms
  P95 Latency:      {:.0}ms
  P99 Latency:      {:.0}ms

🔌 SYSTEM HEALTH
  Services Healthy: {}/{}
  Uptime:           {:.1}%

🎯 AVERAGES
  Avg Confidence:   {:.1}%
  Avg Hold Time:    {:.0}s
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"#,
            report.period_start,
            report.period_end,
            report.total_trades,
            report.win_rate * 100.0,
            report.profit_factor,
            report.total_pnl,
            report.avg_win,
            report.avg_loss,
            report.largest_win,
            report.largest_loss,
            report.avg_return_per_trade_pct,
            report.sharpe_ratio,
            report.sortino_ratio,
            report.calmar_ratio,
            report.max_drawdown_pct,
            report.total_pipeline_runs,
            report.pipeline_p50_ms,
            report.pipeline_p95_ms,
            report.pipeline_p99_ms,
            report.services_healthy,
            report.services_total,
            report.system_uptime_pct,
            report.avg_confidence * 100.0,
            report.avg_holding_time_secs,
        )
    }
}

// ── Statistics Helpers ────────────────────────────────────────────────────────

fn compute_sharpe_sortino(returns: &[f64]) -> (f64, f64) {
    if returns.len() < 2 {
        return (0.0, 0.0);
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;

    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let std_dev = variance.sqrt();

    let downside: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).copied().collect();
    let down_dev = if !downside.is_empty() {
        let down_var =
            downside.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / downside.len() as f64;
        down_var.sqrt()
    } else {
        0.001 // Avoid division by zero
    };

    let sharpe = if std_dev > 0.001 {
        (mean / std_dev) * (252.0_f64).sqrt() // Annualize (assuming daily returns)
    } else {
        0.0
    };

    let sortino = if down_dev > 0.001 {
        (mean / down_dev) * (252.0_f64).sqrt()
    } else if mean > 0.0 {
        999.0 // No downside = excellent
    } else {
        0.0
    };

    (sharpe, sortino)
}

fn compute_max_drawdown(returns: &[f64]) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let mut peak = 0.0_f64;
    let mut cumulative = 0.0_f64;
    let mut max_dd = 0.0_f64;

    for &r in returns {
        cumulative += r;
        if cumulative > peak {
            peak = cumulative;
        }
        let dd = (peak - cumulative) / peak.abs().max(1.0);
        if dd > max_dd {
            max_dd = dd;
        }
    }
    max_dd
}

fn percentile(sorted_data: &[f64], p: f64) -> f64 {
    if sorted_data.is_empty() {
        return 0.0;
    }
    if sorted_data.len() == 1 {
        return sorted_data[0];
    }
    let k = (p / 100.0) * (sorted_data.len() - 1) as f64;
    let f = k.floor() as usize;
    let c = k.ceil() as usize;
    if f == c {
        sorted_data[f]
    } else {
        let d0 = sorted_data[f] * (c as f64 - k);
        let d1 = sorted_data[c] * (k - f as f64);
        d0 + d1
    }
}

// ── Database Initialization ───────────────────────────────────────────────────

fn init_database(path: &str) -> Result<Connection, rusqlite::Error> {
    let expanded = shellexpand(path);
    if let Some(parent) = std::path::Path::new(&expanded).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let db = Connection::open(&expanded)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS trade_outcomes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_micros INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            direction TEXT NOT NULL,
            entry_price REAL NOT NULL,
            exit_price REAL NOT NULL,
            pnl REAL NOT NULL,
            pnl_pct REAL NOT NULL,
            confidence REAL NOT NULL,
            win INTEGER NOT NULL,
            holding_time_secs INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_trade_ts ON trade_outcomes(timestamp_micros DESC);
        CREATE INDEX IF NOT EXISTS idx_trade_win ON trade_outcomes(win);

        CREATE TABLE IF NOT EXISTS latency_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_micros INTEGER NOT NULL,
            component TEXT NOT NULL,
            duration_ms REAL NOT NULL,
            symbol TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_latency_ts ON latency_samples(timestamp_micros DESC);
        CREATE INDEX IF NOT EXISTS idx_latency_comp ON latency_samples(component);

        CREATE TABLE IF NOT EXISTS system_health (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_micros INTEGER NOT NULL,
            service TEXT NOT NULL,
            healthy INTEGER NOT NULL,
            latency_ms REAL,
            error_message TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_health_ts ON system_health(timestamp_micros DESC);
        CREATE INDEX IF NOT EXISTS idx_health_svc ON system_health(service);

        CREATE TABLE IF NOT EXISTS pipeline_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp_micros INTEGER NOT NULL,
            symbol TEXT NOT NULL,
            action TEXT NOT NULL,
            total_duration_ms REAL NOT NULL,
            layers_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_pipeline_ts ON pipeline_runs(timestamp_micros DESC);
        CREATE INDEX IF NOT EXISTS idx_pipeline_action ON pipeline_runs(action);

        CREATE VIEW IF NOT EXISTS metrics_summary AS
        SELECT
            (SELECT COUNT(*) FROM trade_outcomes) as total_trades,
            (SELECT COUNT(*) FROM trade_outcomes WHERE win = 1) as winning_trades,
            (SELECT COALESCE(SUM(pnl), 0) FROM trade_outcomes) as total_pnl,
            (SELECT COALESCE(AVG(duration_ms), 0) FROM latency_samples WHERE component = 'pipeline') as avg_pipeline_ms,
            (SELECT COUNT(*) FROM system_health WHERE healthy = 1) as healthy_checks,
            (SELECT COUNT(*) FROM system_health) as total_checks;"
    )?;

    info!("[Metrics] Database initialized at {}", expanded);
    Ok(db)
}

// ── Config Loading ───────────────────────────────────────────────────────────

fn load_alert_config(path: &str) -> AlertConfig {
    let expanded = shellexpand(path);
    if std::path::Path::new(&expanded).exists() {
        match std::fs::read_to_string(&expanded) {
            Ok(content) => match toml::from_str::<AlertConfig>(&content) {
                Ok(config) => {
                    info!("[Metrics] Loaded alert config from {}", expanded);
                    return config;
                }
                Err(e) => {
                    warn!(
                        "[Metrics] Failed to parse {}: {}. Using defaults.",
                        expanded, e
                    );
                }
            },
            Err(e) => {
                warn!(
                    "[Metrics] Failed to read {}: {}. Using defaults.",
                    expanded, e
                );
            }
        }
    }
    info!("[Metrics] No alert config found. Using defaults.");
    AlertConfig::default()
}

fn write_default_config(path: &str) {
    let expanded = shellexpand(path);
    if let Some(parent) = std::path::Path::new(&expanded).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let config = AlertConfig::default();
    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    let content = format!(
        "# tredo-metrics alert configuration\n\
        # Configure alert channels and thresholds.\n\
        # This file is READ-ONLY at startup.\n\n\
        {}\n\n\
        # Example Slack webhook:\n\
        # slack_webhook_url = \"https://hooks.slack.com/services/T.../B.../xxx\"\n\n\
        # Example Telegram (overrides Notifier defaults):\n\
        # telegram_bot_token = \"123456:ABC-DEF...\"\n\
        # telegram_chat_id = \"-123456789\"\n\n\
        # Example SMTP (email):\n\
        # smtp_host = \"smtp.gmail.com\"\n\
        # smtp_port = 587\n\
        # smtp_username = \"your-email@gmail.com\"\n\
        # smtp_password = \"your-app-password\"\n\
        # smtp_from = \"tredo@yourdomain.com\"\n\
        # smtp_to = \"ops@yourdomain.com\"\n",
        toml_str
    );

    match std::fs::write(&expanded, &content) {
        Ok(()) => info!("[Metrics] Default config written to {}", expanded),
        Err(e) => warn!("[Metrics] Could not write default config: {}", e),
    }
}

fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home)
                .join(rest)
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

// ── HTTP Handlers ─────────────────────────────────────────────────────────────

async fn event_handler(
    State(state): State<Arc<MetricsState>>,
    Json(event): Json<MetricsEvent>,
) -> StatusCode {
    info!(
        "[Metrics] Received event: {:?}",
        std::mem::discriminant(&event)
    );

    if let Err(e) = state.ingest_event(&event) {
        error!("[Metrics] Failed to store event: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    // Run alert checks (non-blocking — errors are logged, not returned)
    if let Err(e) = state.check_alerts(&event) {
        error!("[Metrics] Alert check failed: {}", e);
    }

    StatusCode::OK
}

async fn metrics_handler(State(state): State<Arc<MetricsState>>) -> String {
    state.format_prometheus()
}

async fn report_handler(State(state): State<Arc<MetricsState>>) -> Json<PerformanceReport> {
    let report = state.compute_performance(Utc::now().timestamp_micros() - 7 * 86_400 * 1_000_000);
    Json(report)
}

async fn report_text_handler(State(state): State<Arc<MetricsState>>) -> String {
    let report = state.generate_weekly_report();
    state.format_report_text(&report)
}

async fn report_trigger_handler(State(state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    let report = state.generate_weekly_report();
    let text = state.format_report_text(&report);

    // Deliver report via all configured channels
    state.send_alert("Weekly Performance Report", &text);

    Json(serde_json::json!({
        "status": "delivered",
        "report": report,
    }))
}

async fn status_handler(State(state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    let uptime_secs = (Utc::now().timestamp_micros() - state.startup_time) / 1_000_000;
    let recent_perf = state.compute_performance(Utc::now().timestamp_micros() - 3600 * 1_000_000);

    Json(serde_json::json!({
        "status": "running",
        "uptime_seconds": uptime_secs,
        "recent_trades": recent_perf.total_trades,
        "recent_win_rate": recent_perf.win_rate,
        "recent_pnl": recent_perf.total_pnl,
        "pipeline_p50_ms": recent_perf.pipeline_p50_ms,
        "pipeline_p99_ms": recent_perf.pipeline_p99_ms,
        "timestamp_micros": Utc::now().timestamp_micros(),
    }))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tredo_metrics=info".into()),
        )
        .init();

    println!("╔══════════════════════════════════════════════════════╗");
    println!(
        "║   tredo-metrics v{}                                 ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("║   Multi-Layer Monitoring & Alerting                 ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    // Parse CLI args
    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| CONFIG_PATH.to_string());
    let db_path = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| DB_PATH.to_string());

    // Load alert config
    let alert_config = load_alert_config(&config_path);
    let expanded_config = shellexpand(&config_path);
    if !std::path::Path::new(&expanded_config).exists() {
        write_default_config(&config_path);
    }

    // Initialize database
    let db = match init_database(&db_path) {
        Ok(db) => db,
        Err(e) => {
            error!("[Metrics] Failed to initialize database: {}", e);
            std::process::exit(1);
        }
    };

    let state = Arc::new(MetricsState::new(db, alert_config).expect("Failed to initialize state"));

    println!("[Metrics] 🌐 HTTP server on port {}", HTTP_PORT);
    println!("[Metrics]    POST /event         — Submit metrics event (trade_outcome, latency_sample, system_health, pipeline_run)");
    println!("[Metrics]    GET  /metrics        — Prometheus-formatted metrics");
    println!("[Metrics]    GET  /report         — Weekly performance report (JSON)");
    println!("[Metrics]    GET  /report/text    — Weekly performance report (formatted text)");
    println!(
        "[Metrics]    POST /report/trigger — Generate and deliver weekly report via all channels"
    );
    println!("[Metrics]    GET  /status         — Service status & overview");
    println!("[Metrics]    GET  /health         — Alias for /status");
    println!();

    // Build HTTP router
    let app = Router::new()
        .route("/event", post(event_handler))
        .route("/metrics", get(metrics_handler))
        .route("/report", get(report_handler))
        .route("/report/text", get(report_text_handler))
        .route("/report/trigger", post(report_trigger_handler))
        .route("/status", get(status_handler))
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"status": "ok", "service": "tredo-metrics"})) }),
        )
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], HTTP_PORT));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");
    println!("[Metrics] 🌐 Listening on http://{}/", addr);
    println!("[Metrics] 🚀 Ready to collect metrics and monitor performance.");
    println!(
        "[Metrics] 📊 Add this scrape target to Prometheus: http://localhost:{}/metrics",
        HTTP_PORT
    );

    axum::serve(listener, app).await.unwrap();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS trade_outcomes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_micros INTEGER NOT NULL,
                symbol TEXT NOT NULL,
                direction TEXT NOT NULL,
                entry_price REAL NOT NULL,
                exit_price REAL NOT NULL,
                pnl REAL NOT NULL,
                pnl_pct REAL NOT NULL,
                confidence REAL NOT NULL,
                win INTEGER NOT NULL,
                holding_time_secs INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS latency_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_micros INTEGER NOT NULL,
                component TEXT NOT NULL,
                duration_ms REAL NOT NULL,
                symbol TEXT
            );

            CREATE TABLE IF NOT EXISTS system_health (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_micros INTEGER NOT NULL,
                service TEXT NOT NULL,
                healthy INTEGER NOT NULL,
                latency_ms REAL,
                error_message TEXT
            );

            CREATE TABLE IF NOT EXISTS pipeline_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp_micros INTEGER NOT NULL,
                symbol TEXT NOT NULL,
                action TEXT NOT NULL,
                total_duration_ms REAL NOT NULL,
                layers_json TEXT NOT NULL
            );",
        )
        .unwrap();
        db
    }

    fn test_state() -> MetricsState {
        let db = test_db();
        let config = AlertConfig::default();
        MetricsState::new(db, config).unwrap()
    }

    #[test]
    fn test_store_trade_outcome() {
        let state = test_state();
        let event = MetricsEvent::TradeOutcome(TradeOutcomeEvent {
            symbol: "BTC".to_string(),
            direction: "BUY".to_string(),
            entry_price: 50000.0,
            exit_price: 51000.0,
            pnl: 300.0,
            pnl_pct: 2.0,
            confidence: 0.8,
            win: true,
            holding_time_secs: 3600,
            timestamp_micros: Utc::now().timestamp_micros(),
        });

        state.ingest_event(&event).unwrap();

        let db = state.db.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM trade_outcomes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_store_latency_sample() {
        let state = test_state();
        let event = MetricsEvent::LatencySample(LatencySampleEvent {
            component: "pipeline".to_string(),
            duration_ms: 1500.0,
            symbol: Some("BTC".to_string()),
            timestamp_micros: Utc::now().timestamp_micros(),
        });

        state.ingest_event(&event).unwrap();

        let db = state.db.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM latency_samples", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_performance_metrics() {
        let state = test_state();

        // Insert some trade data
        let now = Utc::now().timestamp_micros();
        {
            let db = state.db.lock().unwrap();
            db.execute(
                "INSERT INTO trade_outcomes (timestamp_micros, symbol, direction, entry_price, exit_price, pnl, pnl_pct, confidence, win, holding_time_secs)
                 VALUES (?1, 'BTC', 'BUY', 100, 110, 10.0, 10.0, 0.8, 1, 3600)",
                params![now - 1000],
            ).unwrap();
            db.execute(
                "INSERT INTO trade_outcomes (timestamp_micros, symbol, direction, entry_price, exit_price, pnl, pnl_pct, confidence, win, holding_time_secs)
                 VALUES (?1, 'ETH', 'BUY', 100, 90, -10.0, -10.0, 0.7, 0, 1800)",
                params![now - 500],
            ).unwrap();
            db.execute(
                "INSERT INTO trade_outcomes (timestamp_micros, symbol, direction, entry_price, exit_price, pnl, pnl_pct, confidence, win, holding_time_secs)
                 VALUES (?1, 'SOL', 'SELL', 100, 105, 5.0, 5.0, 0.9, 1, 7200)",
                params![now],
            ).unwrap();
        }

        let perf = state.compute_performance(now - 86400 * 1_000_000);

        assert_eq!(perf.total_trades, 3);
        assert_eq!(perf.winning_trades, 2);
        assert_eq!(perf.losing_trades, 1);
        assert!((perf.win_rate - 0.6667).abs() < 0.01);
        assert!((perf.total_pnl - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_percentile() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert!((percentile(&data, 50.0) - 5.5).abs() < 0.1);
        assert!((percentile(&data, 95.0) - 9.5).abs() < 0.1);
        assert!((percentile(&data, 99.0) - 9.9).abs() < 0.1);
    }

    #[test]
    fn test_empty_performance() {
        let state = test_state();
        let perf = state.compute_performance(Utc::now().timestamp_micros());
        assert_eq!(perf.total_trades, 0);
        assert_eq!(perf.win_rate, 0.0);
    }

    #[test]
    fn test_prometheus_format() {
        let state = test_state();
        let output = state.format_prometheus();
        assert!(output.contains("tredo_uptime_seconds"));
        assert!(output.contains("tredo_win_rate"));
        assert!(output.contains("tredo_sharpe_ratio"));
    }
}
