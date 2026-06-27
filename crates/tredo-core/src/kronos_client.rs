use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::time::Duration;

/// Returns true when the given env var is set to a truthy value
/// (`1`, `true`, `yes`, `on`, case-insensitive).
fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

/// OHLCV bar for Kronos input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhlcvBar {
    pub timestamp: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Request payload for Kronos forecast
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KronosForecastRequest {
    pub symbol: String,
    pub ohlcv: Vec<OhlcvBar>,
    pub pred_len: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub sample_count: u32,
}

/// Response from Kronos service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KronosForecastResponse {
    pub symbol: String,
    pub forecasts: Vec<serde_json::Value>,
    pub message: String,
}

/// HTTP client to communicate with the Kronos Forecasting Service
#[derive(Clone)]
pub struct KronosClient {
    client: Client,
    base_url: String,
}

impl KronosClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Call the /forecast endpoint of the Kronos service
    /// Uses a 25-second timeout to accommodate slow forecast model inference.
    pub async fn forecast(
        &self,
        request: KronosForecastRequest,
    ) -> Result<KronosForecastResponse, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/forecast", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&url)
            .json(&request)
            .timeout(Duration::from_secs(25))
            .send()
            .await?
            .error_for_status()?
            .json::<KronosForecastResponse>()
            .await?;
        Ok(response)
    }
}

/// High-level Kronos Forecasting Tool.
/// Combines the fast HTTP client with a standalone subprocess fallback.
#[derive(Clone)]
pub struct KronosForecastTool {
    client: KronosClient,
    cli_script_path: String,
}

impl KronosForecastTool {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: KronosClient::new(base_url),
            cli_script_path: "kronos_service/tool.py".to_string(),
        }
    }

    /// Run the forecast. First tries HTTP, then falls back to CLI subprocess execution.
    pub async fn forecast(
        &self,
        request: KronosForecastRequest,
    ) -> Result<KronosForecastResponse, Box<dyn Error + Send + Sync>> {
        // 1. Try HTTP client first (fast, model kept warm in memory)
        match self.client.forecast(request.clone()).await {
            Ok(resp) => {
                let mut resp = resp;
                resp.message = format!("{} (HTTP)", resp.message);
                return Ok(resp);
            }
            Err(e) => {
                // Allow disabling the subprocess fallback (e.g. in tests or in
                // deployments that don't ship the Python CLI). When disabled, an
                // unreachable HTTP service means "no forecast" rather than
                // silently shelling out to `python3`.
                if env_flag_enabled("TREDO_DISABLE_KRONOS_CLI") {
                    return Err(e);
                }
                println!("[KronosForecastTool] HTTP request failed: {}. Falling back to CLI execution...", e);
            }
        }

        // 2. Fallback: Spawn `python3 kronos_service/tool.py`
        let closes_str = request
            .ohlcv
            .iter()
            .map(|b| b.close.to_string())
            .collect::<Vec<_>>()
            .join(",");

        // Check if file exists relative to execution CWD or find correct path
        let mut script_path = std::path::PathBuf::from(&self.cli_script_path);
        if !script_path.exists() {
            // Try walking up to find workspace root
            if let Ok(cwd) = std::env::current_dir() {
                let mut path = cwd.clone();
                while !path.join("kronos_service").exists() {
                    if let Some(parent) = path.parent() {
                        path = parent.to_path_buf();
                    } else {
                        break;
                    }
                }
                script_path = path.join("kronos_service").join("tool.py");
            }
        }

        let output = std::process::Command::new("python3")
            .arg(&script_path)
            .arg("--closes")
            .arg(&closes_str)
            .arg("--pred-len")
            .arg(request.pred_len.to_string())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "Kronos CLI failed with exit code {}: {}",
                output.status, stderr
            )
            .into());
        }

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let mut cli_resp: KronosForecastResponse = serde_json::from_str(&stdout_str)?;
        cli_resp.symbol = request.symbol;
        cli_resp.message = format!("{} (CLI)", cli_resp.message);
        Ok(cli_resp)
    }
}
