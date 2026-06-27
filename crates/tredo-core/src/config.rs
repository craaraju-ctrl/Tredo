#[derive(Debug, Clone)]
pub struct Config {
    pub initial_balance: f64,
    pub max_position_size: f64,
    pub api_key: String,
    pub api_secret: String,
    pub kronos_service_url: String,

    // === Multi-LLM (populated by ./tredo setup wizard) ===
    pub llm_provider: String, // ollama | openai | anthropic | gemini | other
    pub llm_model: String,
    pub llm_endpoint: String,
    pub llm_api_key: String,

    // Additional provider keys for fallbacks / specialized agents (debate, reflection)
    pub openai_api_key: String,
    pub claude_api_key: String,

    // === Notifications (WhatsApp / Telegram) ===
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
    pub whatsapp_sid: String,
    pub whatsapp_token: String,
    pub whatsapp_from: String,

    // === Real-time Tools ===
    pub ws_enabled: bool,
    pub web_api_addr: String,
    pub ws_port: u16,

    // === News ===
    pub newsapi_key: String,
    pub alpha_vantage_key: String,
    pub finnhub_key: String,
    pub marketaux_key: String,

    // === More free/fremium APIs (research 2026: Polygon for aggs+indicators, FRED for macro metrics, CoinGecko keyless/public for crypto) ===
    pub polygon_api_key: String,
    pub fred_api_key: String,

    // Paper enforcement (set by launcher/setup)
    pub paper_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            initial_balance: 100_000.0,
            max_position_size: 0.05,
            api_key: "DUMMY_API_KEY".to_string(),
            api_secret: "DUMMY_API_SECRET".to_string(),
            kronos_service_url: "http://127.0.0.1:8000".to_string(),

            llm_provider: std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "ollama".to_string()),
            llm_model: std::env::var("LLM_MODEL")
                .unwrap_or_else(|_| "nemotron-3-nano:4b".to_string()),
            llm_endpoint: std::env::var("LLM_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            llm_api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),

            openai_api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            claude_api_key: std::env::var("CLAUDE_API_KEY").unwrap_or_default(),

            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default(),
            telegram_chat_id: std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default(),
            whatsapp_sid: std::env::var("WHATSAPP_SID").unwrap_or_default(),
            whatsapp_token: std::env::var("WHATSAPP_TOKEN").unwrap_or_default(),
            whatsapp_from: std::env::var("WHATSAPP_FROM").unwrap_or_default(),

            ws_enabled: std::env::var("WS_ENABLED")
                .map(|v| v == "true" || v == "Y" || v == "y")
                .unwrap_or(true),
            web_api_addr: std::env::var("WEB_API_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8082".to_string()),
            ws_port: std::env::var("WS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8082),

            newsapi_key: std::env::var("NEWSAPI_KEY").unwrap_or_default(),
            alpha_vantage_key: std::env::var("ALPHA_VANTAGE_KEY").unwrap_or_default(),
            finnhub_key: std::env::var("FINNHUB_KEY").unwrap_or_default(),
            marketaux_key: std::env::var("MARKETAUX_KEY").unwrap_or_default(),

            polygon_api_key: std::env::var("POLYGON_API_KEY").unwrap_or_default(),
            fred_api_key: std::env::var("FRED_API_KEY").unwrap_or_default(),

            paper_mode: std::env::var("PAPER_MODE")
                .map(|v| v != "false")
                .unwrap_or(true),
        }
    }
}

impl Config {
    /// Load from env (populated by `source config/tredo.env` after `./tredo setup`).
    /// Future: also support YAML/JSON file load.
    pub fn load() -> Self {
        Self::default()
    }
}
