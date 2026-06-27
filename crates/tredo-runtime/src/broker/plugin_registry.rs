//! Broker plugin registry — discovers brokers via TOML config and builtin plugins.
//!
//! This is a **discovery and instantiation** layer. The actual broker routing
//! (paper vs live) is handled by `tredo_core::paper_engine::BrokerRegistry`.
//! After instantiating a plugin, you register the resulting adapter with
//! the existing `BrokerRegistry::register_live_broker()`.
//!
//! ## Built-in plugins
//! - `paper` — Virtual money via `PaperEngine` (always available)
//! - `zerodha` — Real trading via Kite Connect v3 (`tredo-broker-zerodha` crate)
//! - `alpaca` — US equities/crypto via Alpaca Markets API v2 (`tredo-broker-alpaca` crate)
//!
//! ## External plugins
//! Drop a `.toml` file in `~/.tredo/plugins/brokers/` with the schema:
//! ```toml
//! [broker]
//! id = "my-broker"
//! display_name = "My Broker"
//! description = "Custom broker integration"
//! implementation = "my-broker"
//! [[broker.config_schema]]
//! key = "api_key"
//! label = "API Key"
//! sensitive = true
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tredo_core::paper_engine::{BrokerAdapter, PaperEngineConfig};

/// A field in a broker's configuration schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub description: String,
}

/// A discovered broker plugin (either builtin or from a TOML file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerPlugin {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub config_schema: Vec<ConfigField>,
    /// For built-in brokers: `"builtin:paper"`, `"builtin:zerodha"`.
    /// For external brokers: the name of the implementation (not yet supported).
    pub implementation: String,
}

/// Key-value configuration passed when instantiating a broker.
#[derive(Debug, Clone, Default)]
pub struct BrokerConfig {
    pub values: HashMap<String, String>,
}

impl BrokerConfig {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|s| s.parse().ok())
    }
    pub fn set(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_string(), value.to_string());
    }
}

/// A handle to an instantiated broker, pairing the plugin metadata with the adapter.
pub struct BrokerHandle {
    pub plugin: BrokerPlugin,
    pub adapter: Box<dyn BrokerAdapter>,
}

/// Discovers and instantiates broker plugins.
///
/// Usage:
/// ```rust,ignore
/// let mgr = BrokerPluginManager::new();
/// for p in mgr.list() { println!("{} — {}", p.id, p.display_name); }
/// let handle = mgr.instantiate("zerodha", &config).await?;
/// registry.register_live_broker(Arc::from(handle.adapter)).await;
/// ```
pub struct BrokerPluginManager {
    plugins: Vec<BrokerPlugin>,
    plugins_dir: PathBuf,
}

impl BrokerPluginManager {
    /// Create a new manager, loading builtin plugins and scanning for external ones.
    pub fn new() -> Self {
        let plugins_dir = std::env::var("TREDO_PLUGINS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/tmp"));
                home.join(".tredo").join("plugins").join("brokers")
            });
        let mut mgr = Self {
            plugins: Vec::new(),
            plugins_dir,
        };
        mgr.load_builtins();
        mgr.discover_external();
        mgr
    }

    /// Load the built-in plugins.
    fn load_builtins(&mut self) {
        self.plugins.push(BrokerPlugin {
            id: "paper".into(),
            display_name: "Paper Trading".into(),
            description: "Simulated trading with virtual money. Always available.".into(),
            config_schema: vec![ConfigField {
                key: "initial_balance".into(),
                label: "Starting Capital".into(),
                sensitive: false,
                default: Some("100000".into()),
                description: "Virtual money to start with".into(),
            }],
            implementation: "builtin:paper".into(),
        });
        self.plugins.push(BrokerPlugin {
            id: "zerodha".into(),
            display_name: "Zerodha Kite (Live)".into(),
            description: "Real trading via Zerodha Kite Connect API. ⚠ Real money.".into(),
            config_schema: vec![
                ConfigField {
                    key: "api_key".into(),
                    label: "API Key".into(),
                    sensitive: true,
                    default: None,
                    description: "From https://kite.zerodha.com/developer/apps".into(),
                },
                ConfigField {
                    key: "api_secret".into(),
                    label: "API Secret".into(),
                    sensitive: true,
                    default: None,
                    description: "Provided when you create a Kite Connect app".into(),
                },
                ConfigField {
                    key: "request_token".into(),
                    label: "Request Token".into(),
                    sensitive: true,
                    default: None,
                    description: "From OAuth callback URL after login".into(),
                },
                ConfigField {
                    key: "max_daily_loss".into(),
                    label: "Max Daily Loss (₹)".into(),
                    sensitive: false,
                    default: Some("1000".into()),
                    description: "Hard circuit breaker. Trading halts if exceeded.".into(),
                },
            ],
            implementation: "builtin:zerodha".into(),
        });

        self.plugins.push(BrokerPlugin {
            id: "upstox".into(),
            display_name: "Upstox (Live)".into(),
            description:
                "Real trading via Upstox API v2. Free Indian discount broker. ⚠ Real money.".into(),
            config_schema: vec![
                ConfigField {
                    key: "client_id".into(),
                    label: "Client ID".into(),
                    sensitive: true,
                    default: None,
                    description: "From https://upstox.com/developer/".into(),
                },
                ConfigField {
                    key: "client_secret".into(),
                    label: "Client Secret".into(),
                    sensitive: true,
                    default: None,
                    description: "Provided when you create an Upstox app".into(),
                },
                ConfigField {
                    key: "redirect_uri".into(),
                    label: "Redirect URI".into(),
                    sensitive: false,
                    default: None,
                    description: "OAuth redirect URI".into(),
                },
                ConfigField {
                    key: "access_token".into(),
                    label: "Access Token".into(),
                    sensitive: true,
                    default: None,
                    description: "Pre-obtained access token (optional)".into(),
                },
            ],
            implementation: "builtin:upstox".into(),
        });

        self.plugins.push(BrokerPlugin {
            id: "angelone".into(),
            display_name: "Angel One (Live)".into(),
            description:
                "Real trading via Angel One SmartAPI. Free Indian discount broker. ⚠ Real money."
                    .into(),
            config_schema: vec![
                ConfigField {
                    key: "api_key".into(),
                    label: "API Key".into(),
                    sensitive: true,
                    default: None,
                    description: "From https://smartapi.angelbroking.com/".into(),
                },
                ConfigField {
                    key: "client_id".into(),
                    label: "Client ID".into(),
                    sensitive: true,
                    default: None,
                    description: "Your Angel One client code".into(),
                },
                ConfigField {
                    key: "pin".into(),
                    label: "Trading PIN".into(),
                    sensitive: true,
                    default: None,
                    description: "Your Angel One trading PIN".into(),
                },
                ConfigField {
                    key: "totp_secret".into(),
                    label: "TOTP Secret".into(),
                    sensitive: true,
                    default: None,
                    description: "TOTP secret for 2FA (base64 encoded)".into(),
                },
            ],
            implementation: "builtin:angelone".into(),
        });

        self.plugins.push(BrokerPlugin {
            id: "5paisa".into(),
            display_name: "5Paisa (Live)".into(),
            description:
                "Real trading via 5Paisa Xstream API. Free Indian discount broker. ⚠ Real money."
                    .into(),
            config_schema: vec![
                ConfigField {
                    key: "app_key".into(),
                    label: "App Key".into(),
                    sensitive: true,
                    default: None,
                    description: "From https://xstream.5paisa.com/dev-docs/".into(),
                },
                ConfigField {
                    key: "encry_key".into(),
                    label: "Encryption Key".into(),
                    sensitive: true,
                    default: None,
                    description: "Your encryption key".into(),
                },
                ConfigField {
                    key: "user_id".into(),
                    label: "User ID".into(),
                    sensitive: true,
                    default: None,
                    description: "Your user ID".into(),
                },
                ConfigField {
                    key: "client_code".into(),
                    label: "Client Code".into(),
                    sensitive: true,
                    default: None,
                    description: "Your client code".into(),
                },
            ],
            implementation: "builtin:5paisa".into(),
        });

        self.plugins.push(BrokerPlugin {
            id: "alpaca".into(),
            display_name: "Alpaca (Paper+Live)".into(),
            description:
                "US equities & crypto via Alpaca Markets API v2. Supports paper (free) and live."
                    .into(),
            config_schema: vec![
                ConfigField {
                    key: "api_key_id".into(),
                    label: "API Key ID".into(),
                    sensitive: true,
                    default: None,
                    description: "From https://app.alpaca.markets/".into(),
                },
                ConfigField {
                    key: "api_secret_key".into(),
                    label: "Secret Key".into(),
                    sensitive: true,
                    default: None,
                    description: "From https://app.alpaca.markets/".into(),
                },
                ConfigField {
                    key: "paper".into(),
                    label: "Paper Trading Mode".into(),
                    sensitive: false,
                    default: Some("true".into()),
                    description: "Use paper API (true) or live API (false)".into(),
                },
            ],
            implementation: "builtin:alpaca".into(),
        });
    }

    /// Scan the plugins directory for `.toml` files and load external plugins.
    fn discover_external(&mut self) {
        if !self.plugins_dir.exists() {
            let _ = std::fs::create_dir_all(&self.plugins_dir);
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&self.plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "toml").unwrap_or(false) {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => match toml::from_str::<BrokerPlugin>(&content) {
                            Ok(plugin) => {
                                tracing::info!(
                                    "Discovered external broker plugin: {} ({})",
                                    plugin.id,
                                    path.display()
                                );
                                self.plugins.push(plugin);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse {}: {}", path.display(), e)
                            }
                        },
                        Err(e) => tracing::warn!("Failed to read {}: {}", path.display(), e),
                    }
                }
            }
        }
    }

    /// List all discovered plugins (builtin + external).
    pub fn list(&self) -> &[BrokerPlugin] {
        &self.plugins
    }

    /// Get a plugin by ID.
    pub fn get(&self, id: &str) -> Option<&BrokerPlugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Instantiate a broker from a plugin ID and configuration.
    ///
    /// Credentials are resolved in order:
    /// 1. Provided `config` values
    /// 2. Environment variables (e.g., `ZERODHA_API_KEY`)
    /// 3. Saved config files (`~/.tredo/{id}.toml`)
    pub async fn instantiate(
        &self,
        id: &str,
        config: &BrokerConfig,
    ) -> Result<BrokerHandle, String> {
        let plugin = self
            .get(id)
            .ok_or_else(|| format!("Unknown broker plugin: {}", id))?;

        // Merge config values: explicit config > env vars > saved file
        let merged = self.merge_config(id, config);

        match plugin.implementation.as_str() {
            "builtin:paper" => {
                let balance = merged.get_f64("initial_balance").unwrap_or(100_000.0);
                let cfg = PaperEngineConfig {
                    initial_balance: balance,
                    ..Default::default()
                };
                let adapter = tredo_core::paper_engine::PaperBroker::new(cfg);
                adapter.connect().await?;
                Ok(BrokerHandle {
                    plugin: plugin.clone(),
                    adapter: Box::new(adapter),
                })
            }
            "builtin:zerodha" => {
                let api_key = merged
                    .get("api_key")
                    .ok_or_else(|| {
                        "Zerodha: api_key required (set in config or ZERODHA_API_KEY env)"
                            .to_string()
                    })?
                    .to_string();
                let api_secret = merged
                    .get("api_secret")
                    .ok_or_else(|| {
                        "Zerodha: api_secret required (set in config or ZERODHA_API_SECRET env)"
                            .to_string()
                    })?
                    .to_string();
                let request_token = merged.get("request_token").unwrap_or("").to_string();
                // Use the existing tredo-broker-zerodha crate
                let broker = tredo_broker_zerodha::ZerodhaKiteBroker::new(
                    &api_key,
                    &api_secret,
                    "https://api.kite.trade",
                    &request_token,
                );
                // Connect (will exchange request_token for access_token)
                broker.connect().await?;
                Ok(BrokerHandle {
                    plugin: plugin.clone(),
                    adapter: Box::new(broker),
                })
            }
            "builtin:upstox" => {
                let client_id = merged
                    .get("client_id")
                    .ok_or_else(|| {
                        "Upstox: client_id required (set in config or UPSTOX_CLIENT_ID env)"
                            .to_string()
                    })?
                    .to_string();
                let client_secret = merged
                    .get("client_secret")
                    .ok_or_else(|| {
                        "Upstox: client_secret required (set in config or UPSTOX_CLIENT_SECRET env)"
                            .to_string()
                    })?
                    .to_string();
                let redirect_uri = merged
                    .get("redirect_uri")
                    .unwrap_or("http://localhost:8080/callback")
                    .to_string();
                let access_token = merged.get("access_token").unwrap_or("").to_string();
                let broker = tredo_broker_upstox::UpstoxBroker::new(
                    &client_id,
                    &client_secret,
                    &redirect_uri,
                    &access_token,
                    false,
                );
                broker.connect().await?;
                Ok(BrokerHandle {
                    plugin: plugin.clone(),
                    adapter: Box::new(broker),
                })
            }
            "builtin:angelone" => {
                let api_key = merged
                    .get("api_key")
                    .ok_or_else(|| {
                        "Angel One: api_key required (set in config or ANGEL_API_KEY env)"
                            .to_string()
                    })?
                    .to_string();
                let client_id = merged
                    .get("client_id")
                    .ok_or_else(|| {
                        "Angel One: client_id required (set in config or ANGEL_CLIENT_ID env)"
                            .to_string()
                    })?
                    .to_string();
                let pin = merged
                    .get("pin")
                    .ok_or_else(|| {
                        "Angel One: pin required (set in config or ANGEL_PIN env)".to_string()
                    })?
                    .to_string();
                let totp_secret = merged.get("totp_secret").map(|s| s.to_string());
                let auth_token = merged.get("auth_token").unwrap_or("").to_string();
                let broker = tredo_broker_angelone::AngelOneBroker::new(
                    &api_key,
                    &client_id,
                    &pin,
                    totp_secret,
                    &auth_token,
                );
                broker.connect().await?;
                Ok(BrokerHandle {
                    plugin: plugin.clone(),
                    adapter: Box::new(broker),
                })
            }
            "builtin:5paisa" => {
                let app_key = merged
                    .get("app_key")
                    .ok_or_else(|| {
                        "5Paisa: app_key required (set in config or FIVEPAISA_APP_KEY env)"
                            .to_string()
                    })?
                    .to_string();
                let encry_key = merged
                    .get("encry_key")
                    .ok_or_else(|| {
                        "5Paisa: encry_key required (set in config or FIVEPAISA_ENCRY_KEY env)"
                            .to_string()
                    })?
                    .to_string();
                let user_id = merged
                    .get("user_id")
                    .ok_or_else(|| {
                        "5Paisa: user_id required (set in config or FIVEPAISA_USER_ID env)"
                            .to_string()
                    })?
                    .to_string();
                let client_code = merged
                    .get("client_code")
                    .ok_or_else(|| {
                        "5Paisa: client_code required (set in config or FIVEPAISA_CLIENT_CODE env)"
                            .to_string()
                    })?
                    .to_string();
                let broker = tredo_broker_5paisa::FivePaisaBroker::new(
                    &app_key,
                    &encry_key,
                    &user_id,
                    &client_code,
                    "",
                );
                broker.connect().await?;
                Ok(BrokerHandle {
                    plugin: plugin.clone(),
                    adapter: Box::new(broker),
                })
            }
            "builtin:alpaca" => {
                let api_key_id = merged
                    .get("api_key_id")
                    .ok_or_else(|| {
                        "Alpaca: api_key_id required (set in config or ALPACA_API_KEY_ID env)"
                            .to_string()
                    })?
                    .to_string();
                let api_secret_key = merged
                    .get("api_secret_key")
                    .ok_or_else(|| {
                        "Alpaca: api_secret_key required (set in config or ALPACA_API_SECRET_KEY env)"
                            .to_string()
                    })?
                    .to_string();
                let paper = merged.get("paper").map(|s| s == "true").unwrap_or(true);
                // Use the existing tredo-broker-alpaca crate
                use tredo_broker_alpaca::AlpacaBroker;
                let broker = AlpacaBroker::new(&api_key_id, &api_secret_key, paper);
                broker.connect().await?;
                Ok(BrokerHandle {
                    plugin: plugin.clone(),
                    adapter: Box::new(broker),
                })
            }
            other => Err(format!("Unknown plugin implementation: {}", other)),
        }
    }

    /// Merge configuration sources: explicit config > env vars > saved file.
    fn merge_config(&self, id: &str, explicit: &BrokerConfig) -> BrokerConfig {
        let mut merged = BrokerConfig::default();

        // 1. Start with saved file config (if any)
        let saved_path = self.saved_config_path(id);
        if saved_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&saved_path) {
                if let Ok(parsed) = toml::from_str::<HashMap<String, String>>(&content) {
                    for (k, v) in parsed {
                        merged.set(&k, &v);
                    }
                }
            }
        }

        // 2. Override with env vars (for sensitive fields)
        let env_map = self.env_map(id);
        for (k, v) in env_map {
            merged.set(&k, &v);
        }

        // 3. Override with explicit config (highest priority)
        for (k, v) in &explicit.values {
            merged.set(k, v);
        }

        merged
    }

    /// Map environment variables to config keys for a broker.
    fn env_map(&self, id: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        match id {
            "zerodha" => {
                if let Ok(v) = std::env::var("ZERODHA_API_KEY") {
                    map.insert("api_key".to_string(), v);
                }
                if let Ok(v) = std::env::var("ZERODHA_API_SECRET") {
                    map.insert("api_secret".to_string(), v);
                }
                if let Ok(v) = std::env::var("ZERODHA_REQUEST_TOKEN") {
                    map.insert("request_token".to_string(), v);
                }
                if let Ok(v) = std::env::var("ZERODHA_MAX_DAILY_LOSS") {
                    map.insert("max_daily_loss".to_string(), v);
                }
            }
            "upstox" => {
                if let Ok(v) = std::env::var("UPSTOX_CLIENT_ID") {
                    map.insert("client_id".to_string(), v);
                }
                if let Ok(v) = std::env::var("UPSTOX_CLIENT_SECRET") {
                    map.insert("client_secret".to_string(), v);
                }
                if let Ok(v) = std::env::var("UPSTOX_REDIRECT_URI") {
                    map.insert("redirect_uri".to_string(), v);
                }
                if let Ok(v) = std::env::var("UPSTOX_ACCESS_TOKEN") {
                    map.insert("access_token".to_string(), v);
                }
            }
            "angelone" => {
                if let Ok(v) = std::env::var("ANGEL_API_KEY") {
                    map.insert("api_key".to_string(), v);
                }
                if let Ok(v) = std::env::var("ANGEL_CLIENT_ID") {
                    map.insert("client_id".to_string(), v);
                }
                if let Ok(v) = std::env::var("ANGEL_PIN") {
                    map.insert("pin".to_string(), v);
                }
                if let Ok(v) = std::env::var("ANGEL_TOTP_SECRET") {
                    map.insert("totp_secret".to_string(), v);
                }
                if let Ok(v) = std::env::var("ANGEL_AUTH_TOKEN") {
                    map.insert("auth_token".to_string(), v);
                }
            }
            "5paisa" => {
                if let Ok(v) = std::env::var("FIVEPAISA_APP_KEY") {
                    map.insert("app_key".to_string(), v);
                }
                if let Ok(v) = std::env::var("FIVEPAISA_ENCRY_KEY") {
                    map.insert("encry_key".to_string(), v);
                }
                if let Ok(v) = std::env::var("FIVEPAISA_USER_ID") {
                    map.insert("user_id".to_string(), v);
                }
                if let Ok(v) = std::env::var("FIVEPAISA_CLIENT_CODE") {
                    map.insert("client_code".to_string(), v);
                }
                if let Ok(v) = std::env::var("FIVEPAISA_ACCESS_TOKEN") {
                    map.insert("access_token".to_string(), v);
                }
            }
            "alpaca" => {
                if let Ok(v) = std::env::var("ALPACA_API_KEY_ID") {
                    map.insert("api_key_id".to_string(), v);
                }
                if let Ok(v) = std::env::var("ALPACA_API_SECRET_KEY") {
                    map.insert("api_secret_key".to_string(), v);
                }
                if let Ok(v) = std::env::var("ALPACA_PAPER") {
                    map.insert("paper".to_string(), v);
                }
            }
            _ => {}
        }
        map
    }

    /// Path to saved config for a broker.
    fn saved_config_path(&self, id: &str) -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".tredo").join(format!("{}.toml", id))
    }

    /// Save broker configuration to `~/.tredo/{id}.toml`.
    pub fn save_config(&self, id: &str, config: &BrokerConfig) -> Result<(), String> {
        let config_dir = self
            .saved_config_path(id)
            .parent()
            .unwrap_or(&self.plugins_dir)
            .to_path_buf();
        std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
        let path = self.saved_config_path(id);
        let toml_str =
            toml::to_string(&config.values).map_err(|e| format!("Failed to serialize: {}", e))?;
        std::fs::write(&path, toml_str).map_err(|e| format!("Failed to write: {}", e))?;
        tracing::info!("Broker config saved to {}", path.display());
        Ok(())
    }
}

impl Default for BrokerPluginManager {
    fn default() -> Self {
        Self::new()
    }
}
