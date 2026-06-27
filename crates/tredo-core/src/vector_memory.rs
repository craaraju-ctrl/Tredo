use crate::LlmExecutor;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    pub episode_id: String,
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub embedding: Vec<f32>,
    pub summary_text: String,
    pub regret_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarResult {
    pub episode_id: String,
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub similarity: f64,
    pub summary_text: String,
    pub regret_score: Option<f64>,
}

#[derive(Debug)]
pub struct VectorMemory {
    client: reqwest::Client,
    base_url: String,
    is_online: bool,
}

impl VectorMemory {
    pub fn new(_db_path: &str) -> Self {
        let base_url =
            env::var("MEMORY_API_URL").unwrap_or_else(|_| "http://localhost:3111".to_string());
        let client = reqwest::Client::new();
        // Check health synchronously with a short timeout using ureq.
        let is_online = ureq::get(&format!("{}/health", base_url))
            .timeout(std::time::Duration::from_millis(10))
            .call()
            .is_ok();
        Self {
            client,
            base_url,
            is_online,
        }
    }

    pub async fn store(
        &mut self,
        episode_id: &str,
        symbol: &str,
        summary_text: &str,
        regret_score: Option<f64>,
        llm: &LlmExecutor,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.is_online {
            return Ok(());
        }
        let embedding_f32 = llm.embed_text(summary_text).await?;
        let embedding_f64: Vec<f64> = embedding_f32.iter().map(|&x| x as f64).collect();

        let mut metadata = HashMap::new();
        metadata.insert("symbol".to_string(), symbol.to_string());
        if let Some(r) = regret_score {
            metadata.insert("regret_score".to_string(), r.to_string());
        }

        let url = format!("{}/records", self.base_url);
        let body = json!({
            "id": episode_id,
            "content": summary_text,
            "content_type": "vector_episode",
            "metadata": metadata,
            "embedding": embedding_f64,
            "tier": "episodic",
            "importance": 0.7
        });

        let resp = self.client.post(&url).json(&body).send().await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("Memory service returned status {}", resp.status()).into())
        }
    }

    pub async fn search(
        &self,
        query_text: &str,
        top_k: usize,
        llm: &LlmExecutor,
    ) -> Result<Vec<SimilarResult>, Box<dyn std::error::Error + Send + Sync>> {
        if !self.is_online {
            return Ok(vec![]);
        }
        let query_embedding_f32 = llm.embed_text(query_text).await?;
        let query_embedding_f64: Vec<f64> = query_embedding_f32.iter().map(|&x| x as f64).collect();
        self.search_by_vector_async(query_embedding_f64.as_slice(), top_k)
            .await
    }

    async fn search_by_vector_async(
        &self,
        query_embedding: &[f64],
        top_k: usize,
    ) -> Result<Vec<SimilarResult>, Box<dyn std::error::Error + Send + Sync>> {
        if !self.is_online {
            return Ok(vec![]);
        }
        let url = format!("{}/search/semantic", self.base_url);
        let body = json!({
            "query_vec": query_embedding,
            "k": top_k
        });

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            return Err(format!("Memory service returned status {}", resp.status()).into());
        }

        #[derive(Deserialize)]
        struct ApiSearchResult {
            record: ApiRecord,
            score: f64,
        }

        #[derive(Deserialize)]
        struct ApiRecord {
            id: String,
            content: String,
            metadata: HashMap<String, String>,
            timestamp: String,
        }

        let results: Vec<ApiSearchResult> = resp.json().await?;
        let mut mapped = Vec::new();
        for r in results {
            let ts = DateTime::parse_from_rfc3339(&r.record.timestamp)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let symbol = r.record.metadata.get("symbol").cloned().unwrap_or_default();
            let regret_score = r
                .record
                .metadata
                .get("regret_score")
                .and_then(|s| s.parse::<f64>().ok());

            mapped.push(SimilarResult {
                episode_id: r.record.id,
                symbol,
                timestamp: ts,
                similarity: r.score,
                summary_text: r.record.content,
                regret_score,
            });
        }
        Ok(mapped)
    }

    pub fn search_by_vector(&self, query_embedding: &[f32], top_k: usize) -> Vec<SimilarResult> {
        if !self.is_online {
            return vec![];
        }
        let query_embedding_f64: Vec<f64> = query_embedding.iter().map(|&x| x as f64).collect();
        let url = format!("{}/search/semantic", self.base_url);
        let body = json!({
            "query_vec": query_embedding_f64,
            "k": top_k
        });

        let resp = match ureq::post(&url).send_json(&body) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        if resp.status() != 200 {
            return vec![];
        }

        #[derive(Deserialize)]
        struct ApiSearchResult {
            record: ApiRecord,
            score: f64,
        }

        #[derive(Deserialize)]
        struct ApiRecord {
            id: String,
            content: String,
            metadata: HashMap<String, String>,
            timestamp: String,
        }

        let results: Vec<ApiSearchResult> = match resp.into_json() {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        let mut mapped = Vec::new();
        for r in results {
            let ts = DateTime::parse_from_rfc3339(&r.record.timestamp)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let symbol = r.record.metadata.get("symbol").cloned().unwrap_or_default();
            let regret_score = r
                .record
                .metadata
                .get("regret_score")
                .and_then(|s| s.parse::<f64>().ok());

            mapped.push(SimilarResult {
                episode_id: r.record.id,
                symbol,
                timestamp: ts,
                similarity: r.score,
                summary_text: r.record.content,
                regret_score,
            });
        }
        mapped
    }

    pub fn len(&self) -> usize {
        if !self.is_online {
            return 0;
        }
        let url = format!("{}/stats", self.base_url);
        let resp = match ureq::get(&url).call() {
            Ok(r) => r,
            Err(_) => return 0,
        };

        #[derive(Deserialize)]
        struct Stats {
            total_with_embeddings: u64,
        }

        if resp.status() == 200 {
            if let Ok(stats) = resp.into_json::<Stats>() {
                return stats.total_with_embeddings as usize;
            }
        }
        0
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
