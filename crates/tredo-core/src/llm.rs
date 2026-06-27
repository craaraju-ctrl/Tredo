use crate::messages::LLMRequest;
use crate::news::NewsItem;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::error::Error;

/// Structured trade decision returned by the LLM.
#[derive(Debug, Clone)]
pub struct LlmTradeDecision {
    /// "BUY", "SELL", or "HOLD"
    pub action: String,
    pub entry: f64,
    pub sl: f64,
    pub tp: f64,
    pub reason: String,
}

impl Default for LlmTradeDecision {
    fn default() -> Self {
        Self {
            action: "HOLD".to_string(),
            entry: 0.0,
            sl: 0.0,
            tp: 0.0,
            reason: "LLM fallback: defaulting to HOLD".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmExecutor {
    pub client: Client,
    pub provider: String, // ollama | openai | anthropic | gemini | other
    pub model: String,
    pub endpoint: String,
    pub api_key: String,
    available_models: Vec<OllamaModel>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size: Option<String>,
    pub modified: Option<String>,
    pub is_local: bool,
}

impl Default for LlmExecutor {
    fn default() -> Self {
        let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "ollama".to_string());
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "nemotron-3-nano:4b".to_string());
        let endpoint =
            std::env::var("LLM_ENDPOINT").unwrap_or_else(|_| "http://localhost:11434".to_string());
        let api_key = std::env::var("LLM_API_KEY").unwrap_or_default();

        println!(
            "[LlmExecutor] 🤖 Provider: {} | Model: {} | Endpoint: {}",
            provider, model, endpoint
        );

        Self {
            client: Client::new(),
            provider,
            model,
            endpoint,
            api_key,
            available_models: Vec::new(),
        }
    }
}

impl LlmExecutor {
    /// Get the current model name
    pub fn get_model(&self) -> String {
        self.model.clone()
    }

    /// Set a new model (local or cloud)
    pub fn set_model(&mut self, model: String) {
        println!(
            "[LlmExecutor] 🔄 Switching model from {} to {}",
            self.model, model
        );
        self.model = model;
    }

    /// Get all available models from Ollama
    pub async fn fetch_available_models(
        &mut self,
    ) -> Result<Vec<OllamaModel>, Box<dyn std::error::Error + Send + Sync>> {
        let base_url = self
            .endpoint
            .replace("/api/generate", "")
            .replace("/api/chat", "");

        let res = self
            .client
            .get(format!("{}/api/tags", base_url))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        if !res.status().is_success() {
            return Err(format!("Failed to fetch models: {}", res.status()).into());
        }

        #[derive(Deserialize)]
        struct OllamaTagsResponse {
            models: Vec<OllamaModelInfo>,
        }

        #[derive(Deserialize)]
        struct OllamaModelInfo {
            name: String,
            size: Option<u64>,
            modified_at: Option<String>,
        }

        let tags_res: OllamaTagsResponse = res.json().await?;

        self.available_models = tags_res
            .models
            .into_iter()
            .map(|m| {
                let size_str = m.size.map(|s| {
                    if s > 1_000_000_000 {
                        format!("{:.1}GB", s as f64 / 1_000_000_000.0)
                    } else if s > 1_000_000 {
                        format!("{:.1}MB", s as f64 / 1_000_000.0)
                    } else {
                        format!("{}B", s)
                    }
                });

                OllamaModel {
                    name: m.name,
                    size: size_str,
                    modified: m.modified_at,
                    is_local: true,
                }
            })
            .collect();

        Ok(self.available_models.clone())
    }

    /// Get cached available models
    pub fn get_available_models(&self) -> Vec<OllamaModel> {
        self.available_models.clone()
    }

    /// Check if Ollama is running
    pub async fn is_ollama_running(&self) -> bool {
        let base_url = self
            .endpoint
            .replace("/api/generate", "")
            .replace("/api/chat", "");
        match self
            .client
            .get(format!("{}/api/tags", base_url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(res) => res.status().is_success(),
            Err(_) => false,
        }
    }
}

impl LlmExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build executor from application config (preferred for SharedState init).
    pub fn from_config(config: &crate::Config) -> Self {
        println!(
            "[LlmExecutor] 🤖 Provider: {} | Model: {} | Endpoint: {}",
            config.llm_provider, config.llm_model, config.llm_endpoint
        );
        Self {
            client: Client::new(),
            provider: config.llm_provider.clone(),
            model: config.llm_model.clone(),
            endpoint: config.llm_endpoint.clone(),
            api_key: config.llm_api_key.clone(),
            available_models: Vec::new(),
        }
    }

    /// Generic prompt execution — multi-provider aware.
    /// - ollama / openai / other (OpenAI-compatible /v1/chat/completions)
    /// - anthropic / gemini have TODO stubs (easy to extend with their specific message formats).
    pub async fn execute(
        &self,
        request: crate::messages::LLMRequest,
    ) -> Result<crate::messages::LLMResponse, Box<dyn Error + Send + Sync>> {
        let provider = self.provider.to_lowercase();

        println!(
            "[LlmExecutor] Sending to provider={} model={}",
            provider, self.model
        );

        match provider.as_str() {
            "ollama" | "openai" | "other" => {
                // OpenAI-compatible chat completions (Ollama supports /v1 too, Groq, Together, Fireworks, etc.)
                let chat_url = if provider == "ollama" {
                    format!(
                        "{}/v1/chat/completions",
                        self.endpoint.trim_end_matches('/')
                    )
                } else {
                    format!("{}/chat/completions", self.endpoint.trim_end_matches('/'))
                };

                let body = json!({
                    "model": self.model,
                    "messages": [
                        {"role": "system", "content": "You are a disciplined professional trader. Be concise and output clear BUY/SELL/HOLD decisions with reasoning."},
                        {"role": "user", "content": request.prompt}
                    ],
                    "stream": false,
                    "temperature": 0.7
                });

                let mut req = self.client.post(&chat_url).json(&body);
                if !self.api_key.is_empty() && provider != "ollama" {
                    req = req.bearer_auth(&self.api_key);
                }

                let res = req.send().await?;
                let status = res.status();
                if !status.is_success() {
                    let err = res.text().await.unwrap_or_default();
                    return Err(format!("{} API error ({}): {}", provider, status, err).into());
                }

                #[derive(Deserialize)]
                struct ChatChoice {
                    message: ChatMessage,
                }
                #[derive(Deserialize)]
                struct ChatMessage {
                    content: String,
                }
                #[derive(Deserialize)]
                struct ChatResponse {
                    choices: Vec<ChatChoice>,
                    _usage: Option<serde_json::Value>,
                }

                let chat: ChatResponse = res.json().await?;
                let content = chat
                    .choices
                    .first()
                    .map(|c| c.message.content.clone())
                    .unwrap_or_default();

                Ok(crate::messages::LLMResponse {
                    content,
                    tokens_used: None, // can parse usage if needed
                })
            }
            "anthropic" => {
                // Full Anthropic (Claude) Messages API
                let url = "https://api.anthropic.com/v1/messages";
                let body = json!({
                    "model": self.model,
                    "max_tokens": 1024,
                    "messages": [ { "role": "user", "content": request.prompt } ]
                });
                let res = self
                    .client
                    .post(url)
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&body)
                    .send()
                    .await?;
                if !res.status().is_success() {
                    let err = res.text().await.unwrap_or_default();
                    return Err(format!("Anthropic error: {}", err).into());
                }
                #[derive(Deserialize)]
                struct AnthropicContent {
                    text: String,
                }
                #[derive(Deserialize)]
                struct AnthropicResponse {
                    content: Vec<AnthropicContent>,
                }
                let anth: AnthropicResponse = res.json().await?;
                let content = anth
                    .content
                    .first()
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                Ok(crate::messages::LLMResponse {
                    content,
                    tokens_used: None,
                })
            }
            "gemini" => {
                // Full Google Gemini generateContent
                let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}", self.model, self.api_key);
                let body = json!({
                    "contents": [ { "parts": [ { "text": request.prompt } ] } ]
                });
                let res = self.client.post(&url).json(&body).send().await?;
                if !res.status().is_success() {
                    let err = res.text().await.unwrap_or_default();
                    return Err(format!("Gemini error: {}", err).into());
                }
                #[derive(Deserialize)]
                struct GeminiPart {
                    text: Option<String>,
                }
                #[derive(Deserialize)]
                struct GeminiContent {
                    parts: Vec<GeminiPart>,
                }
                #[derive(Deserialize)]
                struct GeminiCandidate {
                    content: GeminiContent,
                }
                #[derive(Deserialize)]
                struct GeminiResponse {
                    candidates: Vec<GeminiCandidate>,
                }
                let gem: GeminiResponse = res.json().await?;
                let content = gem
                    .candidates
                    .first()
                    .and_then(|c| c.content.parts.first())
                    .and_then(|p| p.text.clone())
                    .unwrap_or_default();
                Ok(crate::messages::LLMResponse {
                    content,
                    tokens_used: None,
                })
            }
            _ => Err(format!("Unknown LLM provider: {}", provider).into()),
        }
    }

    /// Ask the LLM to produce a structured trade decision (BUY/SELL/HOLD).
    ///
    /// This is the core agentic AI decision function. The LLM receives:
    /// - Real-time market data (price, pivots, confluence)
    /// - Kronos AI forecast summary
    /// - Portfolio risk state
    /// - Upcoming economic calendar events
    /// - Current trading goals / mode
    /// - Multi-timeframe context
    ///
    /// Returns LlmTradeDecision::default() (HOLD) on any error, so the pipeline always continues.
    #[allow(clippy::too_many_arguments)]
    pub async fn ask_for_trade_decision(
        &self,
        symbol: &str,
        price: f64,
        confluence: f64,
        trend: &str,
        pivot: f64,
        r1: f64,
        s1: f64,
        forecast_summary: &str,
        portfolio_heat: f64,
        _session_open: bool,
        consecutive_losses: u32,
        // New agentic context:
        calendar_context: &str, // e.g. "⚠ FOMC rate decision today at 14:00 EST"
        trading_mode: &str,     // e.g. "Normal", "Conservative", "Aggressive"
        daily_goal_context: &str, // e.g. "Daily P&L target: +0.5% | Current: +0.12%"
        multi_tf_context: &str, // e.g. "1h: Bullish pivot at 24300 | 15m: Ranging"
        agent_market_summary: &str, // e.g. "Market conditions: BTC in uptrend..."
        news_context: &str,
        similar_episodes_context: &str, // "── SIMILAR PAST EPISODES ──\n 1. BTC ..."
        patterns_context: &str, // e.g. "── CANDLESTICK PATTERNS ──\n🟢 Bullish Engulfing (75%)"
    ) -> LlmTradeDecision {
        let sl_long = price * 0.990;
        let tp_long = price * 1.025;
        let sl_short = price * 1.010;
        let tp_short = price * 0.975;

        let prompt = format!(
            r#"You are an autonomous 24/7 trading AI agent managing a portfolio.
Analyze the data below and decide BUY, SELL, or HOLD.

── MARKET DATA ──
Symbol: {symbol}
Price: {price:.2}
Trend: {trend}
Confluence: {confluence:.1}%
Pivot: {pivot:.2} | R1: {r1:.2} | S1: {s1:.2}
Kronos Forecast: {forecast_summary}

── MULTI-TIMEFRAME ──
{multi_tf_context}

── PORTFOLIO ──
Heat: {portfolio_heat:.1}%
Consecutive Losses: {consecutive_losses}
Mode: {trading_mode}
{daily_goal_context}

── ECONOMIC CALENDAR ──
{calendar_context}

── NEWS ──
{news_context}

── SIMILAR PAST EPISODES ──
{similar_episodes_context}

── CANDLESTICK PATTERNS ──
{patterns_context}

── AGENT CONTEXT ──
{agent_market_summary}

── RULES ──
1. HOLD if confluence < 60% unless multi-timeframe strongly aligned.
2. BUY: sl below entry, tp above entry. SELL: opposite.
3. R:R must >= 2:1 minimum.
4. HOLD if session closed (crypto 24x7) or consecutive_losses >= 3.
5. {trading_mode} mode: adjust risk and frequency accordingly.
6. Consider economic calendar events before entering.

Suggested SL for BUY={sl_long:.2}, TP={tp_long:.2}
Suggested SL for SELL={sl_short:.2}, TP={tp_short:.2}

Respond ONLY with valid JSON line:
{{"action":"BUY","entry":{price:.2},"sl":{sl_long:.2},"tp":{tp_long:.2},"reason":"Brief reason"}}
"#,
            symbol = symbol,
            price = price,
            forecast_summary = forecast_summary,
            trend = trend,
            confluence = confluence * 100.0,
            pivot = pivot,
            r1 = r1,
            s1 = s1,
            portfolio_heat = portfolio_heat * 100.0,
            consecutive_losses = consecutive_losses,
            sl_long = sl_long,
            tp_long = tp_long,
            sl_short = sl_short,
            tp_short = tp_short,
            calendar_context = calendar_context,
            trading_mode = trading_mode,
            daily_goal_context = daily_goal_context,
            multi_tf_context = multi_tf_context,
            agent_market_summary = agent_market_summary,
            news_context = news_context,
            similar_episodes_context = similar_episodes_context,
            patterns_context = patterns_context,
        );

        println!(
            "[LlmExecutor] 🧠 Agentic decision for {} @ {:.2} | Mode: {} | Calendar: {}",
            symbol,
            price,
            trading_mode,
            if calendar_context.is_empty() {
                "none"
            } else {
                "loaded"
            }
        );

        let request = LLMRequest {
            request_id: format!("trade_decision_{}", symbol),
            agent_role: crate::role::AgentRole::StrategyDecision,
            prompt,
            context: serde_json::json!({}),
            max_tokens: 256,
            temperature: 0.7,
        };

        match self.execute(request).await {
            Ok(response) => {
                println!(
                    "[LlmExecutor] Raw LLM response: {}",
                    response.content.trim()
                );
                Self::parse_llm_trade_decision(&response.content, price)
            }
            Err(e) => {
                println!(
                    "[LlmExecutor] ⚠ LLM request failed: {}. Defaulting to HOLD.",
                    e
                );
                LlmTradeDecision::default()
            }
        }
    }

    /// Ask the LLM to do a post-trade deep reflection on an episode.
    /// Analyzes what went wrong/right, identifies violated assumptions,
    /// and generates a lesson for the agent's procedural memory.
    pub async fn ask_for_reflection(
        &self,
        episode_summary: &str,
        outcome_summary: &str,
    ) -> crate::episode::PostTradeReflection {
        let prompt = format!(
            r#"You are a trading psychologist analysing a recent trade.

EPISODE:
{episode_summary}

OUTCOME:
{outcome_summary}

Analyse this trade and respond with valid JSON only:
{{
  "lesson": "One-sentence lesson learned",
  "violated_assumptions": ["assumption 1", "assumption 2"],
  "regret_score": 0.0-1.0,
  "what_went_wrong": ["issue 1"],
  "what_went_right": ["positive 1"],
  "suggested_rule_change": "optional rule suggestion or null",
  "should_alert": false
}}"#,
            episode_summary = episode_summary,
            outcome_summary = outcome_summary,
        );

        let default_reflection = crate::episode::PostTradeReflection {
            timestamp: chrono::Utc::now(),
            lesson: "Reflection unavailable".to_string(),
            violated_assumptions: vec![],
            regret_score: 0.5,
            what_went_wrong: vec!["Could not analyse".to_string()],
            what_went_right: vec![],
            suggested_rule_change: None,
            should_alert: false,
        };

        let request = LLMRequest {
            request_id: "reflection".to_string(),
            agent_role: crate::role::AgentRole::Reflector,
            prompt,
            context: serde_json::json!({}),
            max_tokens: 512,
            temperature: 0.5,
        };

        let response = match self.execute(request).await {
            Ok(r) => r,
            Err(e) => {
                println!("[LlmExecutor] ⚠ Reflection request failed: {e}");
                return default_reflection;
            }
        };

        let raw = response.content;
        let start = raw.find('{');
        let end = raw.rfind('}');

        if let (Some(s), Some(e)) = (start, end) {
            if s <= e {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw[s..=e]) {
                    let lesson = v["lesson"]
                        .as_str()
                        .unwrap_or("No lesson extracted")
                        .to_string();
                    let assumptions = v["violated_assumptions"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let regret = v["regret_score"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0);
                    let wrong = v["what_went_wrong"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let right = v["what_went_right"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let rule_change = v["suggested_rule_change"]
                        .as_str()
                        .filter(|s| !s.is_empty() && *s != "null")
                        .map(String::from);
                    let alert = v["should_alert"].as_bool().unwrap_or(false);

                    println!(
                        "[LlmExecutor] 📝 Reflection: regret={:.2} lesson={}",
                        regret, lesson
                    );
                    return crate::episode::PostTradeReflection {
                        timestamp: chrono::Utc::now(),
                        lesson,
                        violated_assumptions: assumptions,
                        regret_score: regret,
                        what_went_wrong: wrong,
                        what_went_right: right,
                        suggested_rule_change: rule_change,
                        should_alert: alert,
                    };
                }
            }
        }

        default_reflection
    }

    /// Ask the LLM to review recent high-regret episodes and propose rule changes.
    pub async fn ask_for_meta_review(
        &self,
        episode_summaries: &[String],
        current_rules_summary: &str,
    ) -> serde_json::Value {
        let episodes_text = episode_summaries.join("\n---\n");
        let prompt = format!(
            r#"You are a risk manager reviewing the agent's recent trading mistakes.

HIGH-REGRET EPISODES:
{episodes_text}

CURRENT RULES:
{current_rules_summary}

Analyse these mistakes. What patterns do you see? 
Respond with JSON:
{{
  "pattern": "description of common pattern",
  "suggested_changes": [{{"rule": "max_risk_per_trade", "current_value": 0.01, "suggested_value": 0.008, "reason": "..."}}],
  "risk_assessment": "aggregate risk level",
  "recommendation": "summary recommendation"
}}"#,
            episodes_text = episodes_text,
            current_rules_summary = current_rules_summary,
        );

        let default_val = serde_json::json!({
            "pattern": "No analysis available",
            "suggested_changes": [],
            "risk_assessment": "unknown",
            "recommendation": "No recommendation"
        });

        let request = LLMRequest {
            request_id: "meta_review".to_string(),
            agent_role: crate::role::AgentRole::Reflector,
            prompt,
            context: serde_json::json!({}),
            max_tokens: 512,
            temperature: 0.7,
        };

        let response = match self.execute(request).await {
            Ok(r) => r,
            Err(e) => {
                println!("[LlmExecutor] ⚠ Meta-review request failed: {e}");
                return default_val;
            }
        };

        let raw = response.content;
        let start = raw.find('{');
        let end = raw.rfind('}');

        if let (Some(s), Some(e)) = (start, end) {
            if s <= e {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw[s..=e]) {
                    return v;
                }
            }
        }

        default_val
    }

    /// Generate an embedding vector for the given text using Ollama's /api/embed endpoint.
    /// Returns a normalized vector of f32.
    pub async fn embed_text(
        &self,
        text: &str,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "model": self.model.clone(),
            "input": [text],
        });

        let embed_endpoint = self.endpoint.replace("/api/generate", "/api/embed");

        let res = self
            .client
            .post(&embed_endpoint)
            .json(&body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let err_text = res.text().await.unwrap_or_default();
            return Err(format!("Ollama embed API error {}: {}", status, err_text).into());
        }

        #[derive(serde::Deserialize)]
        struct EmbedResponse {
            embeddings: Vec<Vec<f32>>,
        }

        let embed_res: EmbedResponse = res.json().await?;
        embed_res
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| "Empty embeddings response".into())
    }

    /// Summarize a batch of news headlines for a symbol, extracting sentiment and key risks.
    pub async fn summarize_news(&self, headlines: &[NewsItem], symbol: &str) -> String {
        if headlines.is_empty() {
            return "No news available.".to_string();
        }

        // For very small local models (like nemotron-3-nano:4b), skip the LLM summarization call
        // to avoid long timeouts / spammy failures. The core decision LLM (debate/strategy) still uses it.
        if self.model.to_lowercase().contains("nano")
            || self.model.contains("3b")
            || self.model.contains("4b")
        {
            return format!("Fetched {} headlines for {} (LLM news summary skipped for small local model to keep latency low).", headlines.len(), symbol);
        }

        let headlines_text: String = headlines
            .iter()
            .map(|h| format!("- [{}] {}: {}", h.source, h.title, h.url))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"Summarize the following news headlines for {}.
Extract: overall sentiment (positive/negative/neutral), key risks, and market impact.
Keep it to 2-3 sentences.

NEWS:
{}

SUMMARY:"#,
            symbol, headlines_text
        );

        let request = LLMRequest {
            request_id: format!("news_summary_{}", symbol),
            agent_role: crate::role::AgentRole::MarketIntelligence,
            prompt,
            context: serde_json::json!({}),
            max_tokens: 256,
            temperature: 0.5,
        };

        match self.execute(request).await {
            Ok(response) => response.content.trim().to_string(),
            Err(e) => {
                println!("[LlmExecutor] ⚠ News summarization failed: {e}");
                format!(
                    "Fetched {} headlines, summarization unavailable.",
                    headlines.len()
                )
            }
        }
    }

    /// Parse the JSON trade decision from the LLM response text.
    /// Robust: finds the first `{...}` block, handles extra text.
    pub fn parse_llm_trade_decision(raw: &str, current_price: f64) -> LlmTradeDecision {
        let start = raw.find('{');
        let end = raw.rfind('}');

        if let (Some(s), Some(e)) = (start, end) {
            if s <= e {
                let json_str = &raw[s..=e];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let action = v["action"].as_str().unwrap_or("HOLD").to_uppercase();
                    let entry = v["entry"].as_f64().unwrap_or(current_price);
                    let sl = v["sl"].as_f64().unwrap_or(0.0);
                    let tp = v["tp"].as_f64().unwrap_or(0.0);
                    let reason = v["reason"]
                        .as_str()
                        .unwrap_or("LLM provided no reason")
                        .to_string();

                    let valid = match action.as_str() {
                        "BUY" => sl > 0.0 && tp > 0.0 && sl < entry && tp > entry,
                        "SELL" => sl > 0.0 && tp > 0.0 && sl > entry && tp < entry,
                        _ => true,
                    };

                    if valid {
                        println!(
                            "[LlmExecutor] ✅ Parsed decision: {} entry={:.2} sl={:.2} tp={:.2}",
                            action, entry, sl, tp
                        );
                        return LlmTradeDecision {
                            action,
                            entry,
                            sl,
                            tp,
                            reason,
                        };
                    } else {
                        println!("[LlmExecutor] ⚠ LLM returned invalid SL/TP for {}. Defaulting to HOLD.", action);
                    }
                }
            }
        }

        println!("[LlmExecutor] ⚠ Could not parse LLM JSON response. Defaulting to HOLD.");
        LlmTradeDecision {
            reason: format!("Parse failed for: {}", &raw[..raw.len().min(120)]),
            ..LlmTradeDecision::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::LLMRequest;

    #[tokio::test]
    #[ignore]
    async fn test_ollama_inference() {
        let executor = LlmExecutor::new();
        let request = LLMRequest {
            request_id: "test-req".to_string(),
            agent_role: crate::role::AgentRole::MarketIntelligence,
            prompt: "What is 2 + 2? Answer in one short sentence.".to_string(),
            context: serde_json::json!({}),
            max_tokens: 50,
            temperature: 0.1,
        };
        let result = executor
            .execute(request)
            .await
            .expect("Failed to execute LLM request");
        println!("Response: {}", result.content);
        println!("Tokens used: {:?}", result.tokens_used);
        assert!(!result.content.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_trade_decision() {
        let executor = LlmExecutor::new();
        let decision = executor
            .ask_for_trade_decision(
                "NIFTY",
                24500.0,
                0.75,
                "Bullish",
                24400.0,
                24600.0,
                24200.0,
                "Kronos predicts +0.5% over next 5 candles",
                0.05,
                true,
                0,
                "No high-impact events today",
                "Normal",
                "Daily target: +0.5% | Current: +0.12%",
                "1h: Bullish | 15m: Ranging",
                "Market in steady uptrend",
                "No news",
                "",
                "",
            )
            .await;
        println!(
            "Action: {} | Entry: {:.2} | SL: {:.2} | TP: {:.2}",
            decision.action, decision.entry, decision.sl, decision.tp
        );
        println!("Reason: {}", decision.reason);
    }

    #[test]
    fn test_parse_llm_decision_valid_buy() {
        let raw = r#"{"action":"BUY","entry":24500.0,"sl":24250.0,"tp":25000.0,"reason":"Bullish trend"}"#;
        let d = LlmExecutor::parse_llm_trade_decision(raw, 24500.0);
        assert_eq!(d.action, "BUY");
        assert_eq!(d.entry, 24500.0);
    }

    #[test]
    fn test_parse_llm_decision_invalid_sl() {
        let raw = r#"{"action":"BUY","entry":24500.0,"sl":24800.0,"tp":25000.0,"reason":"Bad SL"}"#;
        let d = LlmExecutor::parse_llm_trade_decision(raw, 24500.0);
        assert_eq!(d.action, "HOLD");
    }

    #[test]
    fn test_parse_llm_decision_garbage() {
        let raw = "Sorry, I cannot process this request right now.";
        let d = LlmExecutor::parse_llm_trade_decision(raw, 24500.0);
        assert_eq!(d.action, "HOLD");
    }
}
