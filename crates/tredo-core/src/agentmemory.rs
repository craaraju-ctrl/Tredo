// agentmemory.rs - Persistent memory client for TREDO agents via agentmemory REST.
// Gives infinite long-term memory across restarts (complements redb/sqlite).
// Usage in debate.rs / reflector.rs:
//   let mem = AgentMemoryClient::new();
//   mem.remember("After debate on BTC: HOLD due to low conviction from nano model + Guardian.", "decision").await?;
//   let past = mem.recall("previous BTC decisions").await?; // inject into LLM prompt

use reqwest::Client;
use serde_json::json;
use std::env;

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
            env::var("AGENTMEMORY_URL").unwrap_or_else(|_| "http://localhost:3111".to_string());
        Self {
            client: Client::new(),
            base_url,
        }
    }

    pub async fn remember(&self, content: &str, mem_type: &str) -> Result<(), String> {
        let url = format!("{}/memory", self.base_url);
        let body = json!({
            "scope": "workspace",
            "memories": [{"type": mem_type, "content": content}]
        });
        self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map(|_| ())
            .map_err(|e| format!("agentmemory remember error: {}", e))
    }

    pub async fn recall(&self, query: &str) -> Result<Vec<String>, String> {
        let url = format!("{}/memory?q={}", self.base_url, urlencoding::encode(query));
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let data: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
        Ok(data
            .into_iter()
            .filter_map(|v| {
                v.get("content")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
            })
            .collect())
    }
}

// Example integration point in autonomous debate/strategy:
// before calling LLM for decision:
// let past_context = mem.recall(&format!("past decisions for {}", symbol)).await.unwrap_or_default();
// include past_context in the prompt to the model for true persistent memory.
