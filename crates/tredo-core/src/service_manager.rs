// ═══════════════════════════════════════════════════════════════════════════════
// Service Manager — Health checks, connection status, auto-reconnection
//
// Manages connectivity to external servers:
//   - LLM Server (Ollama/OpenAI/Anthropic/Gemini)
//   - Kronos Forecast Server (Chronos-Bolt time-series)
//   - (Future: broker APIs, news APIs, etc.)
//
// Provides:
//   - Periodic health check pings
//   - Connection status tracking (healthy/degraded/down)
//   - Consecutive failure counting
//   - Response time tracking
//   - Formatted status for logging and TUI display
// ═══════════════════════════════════════════════════════════════════════════════

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Connection status of a service
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConnectionStatus {
    /// Service is reachable and responding
    Healthy,
    /// Service is reachable but slow or partially responding
    Degraded,
    /// Service is unreachable
    Down,
    /// Service has never been checked
    Unknown,
}

impl ConnectionStatus {
    pub fn label(&self) -> &str {
        match self {
            ConnectionStatus::Healthy => "Healthy",
            ConnectionStatus::Degraded => "Degraded",
            ConnectionStatus::Down => "Down",
            ConnectionStatus::Unknown => "Unknown",
        }
    }

    pub fn is_usable(&self) -> bool {
        matches!(self, ConnectionStatus::Healthy | ConnectionStatus::Degraded)
    }
}

/// Detailed status of a single service
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub status: ConnectionStatus,
    pub endpoint: String,
    pub last_ok: Option<DateTime<Utc>>,
    pub last_error: Option<DateTime<Utc>>,
    pub last_error_message: Option<String>,
    pub consecutive_failures: u32,
    pub response_time_ms: Option<u64>,
    pub response_time_avg_ms: Option<f64>,
    pub response_time_history: Vec<u64>,
    pub checks_total: u32,
}

impl ServiceStatus {
    pub fn new(name: &str, endpoint: &str) -> Self {
        Self {
            name: name.to_string(),
            status: ConnectionStatus::Unknown,
            endpoint: endpoint.to_string(),
            last_ok: None,
            last_error: None,
            last_error_message: None,
            consecutive_failures: 0,
            response_time_ms: None,
            response_time_avg_ms: None,
            response_time_history: Vec::new(),
            checks_total: 0,
        }
    }

    /// Format a one-line status summary for TUI/log display
    pub fn format_short(&self) -> String {
        let symbol = match self.status {
            ConnectionStatus::Healthy => "✅",
            ConnectionStatus::Degraded => "⚠️",
            ConnectionStatus::Down => "❌",
            ConnectionStatus::Unknown => "❓",
        };
        let pct_ok = if self.checks_total > 0 {
            let ok_count = self.checks_total - self.consecutive_failures;
            ok_count as f64 / self.checks_total as f64 * 100.0
        } else {
            0.0
        };
        let latency = self
            .response_time_avg_ms
            .map(|ms| format!("{:.0}ms", ms))
            .unwrap_or_else(|| "?".to_string());

        format!(
            "{} {} | {} | uptime {:.0}% | {} avg",
            symbol,
            self.name,
            self.status.label(),
            pct_ok,
            latency
        )
    }

    /// Full multi-line status for logs
    pub fn format_detailed(&self) -> String {
        let mut lines = vec![
            format!("╔══ {} ══╗", self.name),
            format!("║ Status: {} ({})", self.status.label(), self.endpoint),
        ];
        if let Some(ok) = self.last_ok {
            lines.push(format!("║ Last OK: {}", ok.format("%H:%M:%S UTC")));
        }
        if let Some(err) = self.last_error {
            lines.push(format!("║ Last Error: {}", err.format("%H:%M:%S UTC")));
        }
        if let Some(msg) = &self.last_error_message {
            lines.push(format!("║ Error: {}", msg));
        }
        lines.push(format!(
            "║ Checks: {} | Failures: {}",
            self.checks_total, self.consecutive_failures
        ));
        if let Some(ms) = self.response_time_avg_ms {
            lines.push(format!("║ Avg Response: {:.0}ms", ms));
        }
        lines.push("╚════════════════╝".to_string());
        lines.join("\n")
    }
}

/// Central service manager — checks health, tracks status, provides formatted output
#[derive(Debug, Clone)]
pub struct ServiceManager {
    services: Arc<RwLock<HashMap<String, ServiceStatus>>>,
    client: reqwest::Client,
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceManager {
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .user_agent("tredo-service-manager/1.0")
                .build()
                .unwrap_or_default(),
        }
    }

    /// Register a service to be tracked
    pub async fn register_service(&self, name: &str, endpoint: &str) {
        let mut services = self.services.write().await;
        services.insert(name.to_string(), ServiceStatus::new(name, endpoint));
        println!("[ServiceManager] 📝 Registered: {} @ {}", name, endpoint);
    }

    /// Register multiple services at once
    pub async fn register_services(&self, services: Vec<(&str, &str)>) {
        for (name, endpoint) in services {
            self.register_service(name, endpoint).await;
        }
    }

    /// Get the current status of all registered services
    pub async fn get_all_statuses(&self) -> HashMap<String, ServiceStatus> {
        self.services.read().await.clone()
    }

    /// Get the current status of a single service
    pub async fn get_status(&self, name: &str) -> Option<ServiceStatus> {
        self.services.read().await.get(name).cloned()
    }

    /// Check if all critical services are healthy
    pub async fn all_critical_healthy(&self, critical: &[&str]) -> bool {
        let services = self.services.read().await;
        critical.iter().all(|name| {
            services
                .get(*name)
                .map(|s| s.status.is_usable())
                .unwrap_or(false)
        })
    }

    /// Check health of a single service by pinging its endpoint
    pub async fn check_health(&self, name: &str) {
        let endpoint = {
            let services = self.services.read().await;
            services.get(name).map(|s| s.endpoint.clone())
        };

        let endpoint = match endpoint {
            Some(e) => e,
            None => {
                println!("[ServiceManager] ⚠ Unknown service: {}", name);
                return;
            }
        };

        // Skip pinging services with empty endpoints (e.g. paper broker with no external API).
        // Set response times to 0 to avoid "?ms" display in the TUI.
        if endpoint.is_empty() {
            let mut services = self.services.write().await;
            if let Some(status) = services.get_mut(name) {
                status.status = ConnectionStatus::Healthy;
                status.checks_total += 1;
                status.response_time_ms = Some(0);
                status.response_time_avg_ms = Some(match status.response_time_avg_ms {
                    Some(avg) => avg * 0.7 + 0.0,
                    None => 0.0,
                });
                status.last_ok = Some(Utc::now());
                status.consecutive_failures = 0;
            }
            return;
        }

        let start = std::time::Instant::now();
        let result = self.ping_endpoint(name, &endpoint).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let mut services = self.services.write().await;
        if let Some(status) = services.get_mut(name) {
            status.checks_total += 1;
            status.response_time_ms = Some(elapsed_ms);

            // Update rolling avg (EMA)
            status.response_time_avg_ms = Some(match status.response_time_avg_ms {
                Some(avg) => avg * 0.7 + elapsed_ms as f64 * 0.3,
                None => elapsed_ms as f64,
            });

            // Maintain rolling history (last 10 response times)
            status.response_time_history.push(elapsed_ms);
            if status.response_time_history.len() > 10 {
                status.response_time_history.remove(0);
            }

            match result {
                Ok(()) => {
                    status.status = ConnectionStatus::Healthy;
                    status.last_ok = Some(Utc::now());
                    status.consecutive_failures = 0;
                    status.last_error_message = None;
                }
                Err(e) => {
                    status.consecutive_failures += 1;
                    status.last_error = Some(Utc::now());
                    let err_msg = e.clone();
                    status.last_error_message = Some(e);

                    // Degrade or mark down based on consecutive failures
                    if status.consecutive_failures >= 3 {
                        status.status = ConnectionStatus::Down;
                    } else {
                        status.status = ConnectionStatus::Degraded;
                    }

                    println!(
                        "[ServiceManager] ⚠ {} health check FAILED ({}/3): {}",
                        name, status.consecutive_failures, err_msg
                    );
                }
            }
        }
    }

    /// Run health checks on all registered services
    pub async fn run_all_health_checks(&self) {
        let names: Vec<String> = {
            let services = self.services.read().await;
            services.keys().cloned().collect()
        };

        for name in &names {
            self.check_health(name).await;
        }
    }

    /// Periodic health check loop — runs every `interval_secs` seconds
    pub async fn start_health_loop(self, interval_secs: u64) {
        println!(
            "[ServiceManager] 🔄 Starting health check loop (every {}s)",
            interval_secs
        );

        // Run initial checks immediately
        self.run_all_health_checks().await;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            self.run_all_health_checks().await;
        }
    }

    /// Ping a service endpoint to check if it's alive
    /// Handles different endpoint types:
    ///   - LLM (Ollama): GET /api/tags
    ///   - LLM (OpenAI): GET /v1/models
    ///   - Kronos: GET /health
    ///   - Generic: HEAD /
    async fn ping_endpoint(&self, name: &str, endpoint: &str) -> Result<(), String> {
        let base = endpoint.trim_end_matches('/');

        // Determine health endpoint based on service type
        let health_url = if name.to_lowercase().contains("kronos") {
            format!("{}/health", base)
        } else if name.to_lowercase().contains("llm") || name.to_lowercase().contains("ollama") {
            format!("{}/api/tags", base)
        } else if name.to_lowercase().contains("openai") {
            format!("{}/v1/models", base)
        } else {
            format!("{}/", base)
        };

        let response = self
            .client
            .get(&health_url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| format!("Connection failed: {}", e))?;

        let status = response.status();
        // 2xx = healthy. 4xx = server reachable but needs auth (e.g., broker APIs).
        // Only 5xx and network errors count as unhealthy.
        if status.is_success() || status.is_client_error() {
            Ok(())
        } else {
            Err(format!("HTTP {}", status))
        }
    }

    /// Print a formatted status board of all services
    pub async fn print_status_board(&self) {
        let services = self.services.read().await;
        if services.is_empty() {
            println!("[ServiceManager] No services registered.");
            return;
        }

        println!("\n╔══ SERVER STATUS BOARD ══╗");
        for status in services.values() {
            println!("║  {}", status.format_short());
        }
        println!("╚══════════════════════════╝\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_status() {
        let mgr = ServiceManager::new();
        mgr.register_service("ollama", "http://localhost:11434")
            .await;
        mgr.register_service("kronos", "http://127.0.0.1:8000")
            .await;

        let statuses = mgr.get_all_statuses().await;
        assert_eq!(statuses.len(), 2);
        assert_eq!(
            statuses.get("ollama").unwrap().status,
            ConnectionStatus::Unknown
        );
        assert_eq!(
            statuses.get("kronos").unwrap().endpoint,
            "http://127.0.0.1:8000"
        );
    }

    #[test]
    fn test_status_labels() {
        assert_eq!(ConnectionStatus::Healthy.label(), "Healthy");
        assert_eq!(ConnectionStatus::Degraded.label(), "Degraded");
        assert_eq!(ConnectionStatus::Down.label(), "Down");
        assert_eq!(ConnectionStatus::Unknown.label(), "Unknown");
    }

    #[test]
    fn test_status_is_usable() {
        assert!(ConnectionStatus::Healthy.is_usable());
        assert!(ConnectionStatus::Degraded.is_usable());
        assert!(!ConnectionStatus::Down.is_usable());
        assert!(!ConnectionStatus::Unknown.is_usable());
    }

    #[test]
    fn test_format_short() {
        let mut status = ServiceStatus::new("ollama", "http://localhost:11434");
        status.status = ConnectionStatus::Healthy;
        status.checks_total = 10;
        status.consecutive_failures = 0;
        status.response_time_avg_ms = Some(45.0);

        let short = status.format_short();
        assert!(short.contains("✅"));
        assert!(short.contains("ollama"));
        assert!(short.contains("Healthy"));
    }

    #[test]
    fn test_format_short_down() {
        let mut status = ServiceStatus::new("kronos", "http://127.0.0.1:8000");
        status.status = ConnectionStatus::Down;
        status.checks_total = 5;
        status.consecutive_failures = 3;

        let short = status.format_short();
        assert!(short.contains("❌"));
        assert!(short.contains("kronos"));
        assert!(short.contains("40%")); // 2/5 checks passed = 40%
    }
}
