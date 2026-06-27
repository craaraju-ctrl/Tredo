// agentmemory.rs - Persistent memory client for TREDO agents via agentmemory REST.
// Gives infinite long-term memory across restarts (complements redb/sqlite).
// Usage in debate.rs / reflector.rs:
//   let mem = AgentMemoryClient::new();
//   mem.remember("After debate on BTC: HOLD due to low conviction from nano model + Guardian.", "decision").await?;
//   let past = mem.recall("previous BTC decisions").await?; // inject into LLM prompt

use reqwest::Client;
use serde_json::json;
use std::env;

/// Client for the agentic-memory HTTP API (the production-grade memory module
/// running on port 3111). Replaces direct redb/episode_store calls with
/// tiered, graph-aware, consolidated long-term memory.
pub struct AgentMemoryClient {
    client: Client,
    base_url: String,
}

impl Default for AgentMemoryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentMemoryClient {
    pub fn new() -> Self {
        let base_url =
            env::var("MEMORY_API_URL").unwrap_or_else(|_| "http://localhost:3111".to_string());
        Self {
            client: Client::new(),
            base_url,
        }
    }

    /// Store a memory record in the episodic tier. Returns the record ID.
    pub async fn remember(&self, content: &str, content_type: &str) -> Result<String, String> {
        let url = format!("{}/records", self.base_url);
        let body = json!({
            "content": content,
            "content_type": content_type,
            "tier": "episodic",
            "importance": 0.7
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("memory remember error: {}", e))?;
        let status = resp.status();
        if status.as_u16() == 201 {
            let id = resp.text().await.unwrap_or_default();
            Ok(id.trim_matches('"').to_string())
        } else {
            Err(format!("memory remember HTTP {}", status))
        }
    }

    /// Search memory for relevant content matching a query via smart search.
    /// Returns a list of content strings for injection into LLM prompts.
    pub async fn recall(&self, query: &str) -> Result<Vec<String>, String> {
        let url = format!(
            "{}/search/smart?q={}",
            self.base_url,
            urlencoding::encode(query)
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("memory recall error: {}", e))?;
        let data: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
        Ok(data
            .into_iter()
            .filter_map(|v| {
                v.get("record")
                    .and_then(|r| r.get("content"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
            })
            .collect())
    }

    /// Get memory system health stats.
    pub async fn health(&self) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map_err(|e| format!("memory health error: {}", e))?;
        resp.json().await.map_err(|e| e.to_string())
    }

    /// Get storage stats with tier breakdown.
    pub async fn stats(&self) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .get(format!("{}/stats", self.base_url))
            .send()
            .await
            .map_err(|e| format!("memory stats error: {}", e))?;
        resp.json().await.map_err(|e| e.to_string())
    }

    /// Trigger a consolidation cycle.
    pub async fn consolidate(&self) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .post(format!("{}/consolidate", self.base_url))
            .send()
            .await
            .map_err(|e| format!("memory consolidate error: {}", e))?;
        resp.json().await.map_err(|e| e.to_string())
    }

    /// Trigger an evolution (sleep-time) cycle.
    pub async fn evolve(&self) -> Result<serde_json::Value, String> {
        let resp = self
            .client
            .post(format!("{}/evolve", self.base_url))
            .send()
            .await
            .map_err(|e| format!("memory evolve error: {}", e))?;
        resp.json().await.map_err(|e| e.to_string())
    }

    /// Promote a record to a higher memory tier.
    pub async fn promote(&self, id: &str, tier: &str) -> Result<(), String> {
        let resp = self
            .client
            .post(format!("{}/tiers/promote/{}/{}", self.base_url, id, tier))
            .send()
            .await
            .map_err(|e| format!("memory promote error: {}", e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("memory promote HTTP {}", resp.status()))
        }
    }
}

// Example integration point in autonomous debate/strategy:
// before calling LLM for decision:
// let past_context = mem.recall(&format!("past decisions for {}", symbol)).await.unwrap_or_default();
// include past_context in the prompt to the model for true persistent memory.
