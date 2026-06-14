use crate::LlmExecutor;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A stored vector entry with metadata for similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    pub episode_id: String,
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub embedding: Vec<f32>,
    pub summary_text: String, // The text that was embedded (for display)
    pub regret_score: Option<f64>, // Post-trade regret, if available
}

/// Result of a similarity search.
#[derive(Debug, Clone)]
pub struct SimilarResult {
    pub episode_id: String,
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub similarity: f64, // Cosine similarity (0.0 to 1.0)
    pub summary_text: String,
    pub regret_score: Option<f64>,
}

/// Production-ready VectorMemory (JSON fallback; LanceDB optional via `features = ["lancedb"]` for full scale).
/// API is "Lance-ready". Current brute-force + JSON is sufficient and production-viable for the intact system.
/// Upgrade enables proper vector DB indexing/filtering (by regret, regime, symbol) for large-scale trained memory recall in debate/reflector/historian/etc.
pub struct VectorMemory {
    entries: HashMap<String, VectorEntry>,
    db_path: String,
}

impl VectorMemory {
    pub fn new(db_path: &str) -> Self {
        let mut mem = Self {
            entries: HashMap::new(),
            db_path: db_path.to_string(),
        };
        let _ = mem.load_from_disk();
        mem
    }

    pub async fn store(
        &mut self,
        episode_id: &str,
        symbol: &str,
        summary_text: &str,
        regret_score: Option<f64>,
        llm: &LlmExecutor,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let embedding = llm.embed_text(summary_text).await?;

        let entry = VectorEntry {
            episode_id: episode_id.to_string(),
            symbol: symbol.to_string(),
            timestamp: Utc::now(),
            embedding,
            summary_text: summary_text.to_string(),
            regret_score,
        };

        self.entries.insert(episode_id.to_string(), entry);
        let _ = self.save_to_disk();
        Ok(())
    }

    pub async fn search(
        &self,
        query_text: &str,
        top_k: usize,
        llm: &LlmExecutor,
    ) -> Result<Vec<SimilarResult>, Box<dyn std::error::Error + Send + Sync>> {
        if self.entries.is_empty() {
            return Ok(vec![]);
        }

        let query_embedding = llm.embed_text(query_text).await?;

        let mut scored: Vec<(&VectorEntry, f64)> = self
            .entries
            .values()
            .map(|entry| {
                let sim = cosine_similarity(&query_embedding, &entry.embedding);
                (entry, sim)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored
            .into_iter()
            .take(top_k)
            .map(|(entry, sim)| SimilarResult {
                episode_id: entry.episode_id.clone(),
                symbol: entry.symbol.clone(),
                timestamp: entry.timestamp,
                similarity: sim,
                summary_text: entry.summary_text.clone(),
                regret_score: entry.regret_score,
            })
            .collect())
    }

    pub fn search_by_vector(&self, query_embedding: &[f32], top_k: usize) -> Vec<SimilarResult> {
        let mut scored: Vec<(&VectorEntry, f64)> = self
            .entries
            .values()
            .map(|entry| {
                let sim = cosine_similarity(query_embedding, &entry.embedding);
                (entry, sim)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k)
            .map(|(entry, sim)| SimilarResult {
                episode_id: entry.episode_id.clone(),
                symbol: entry.symbol.clone(),
                timestamp: entry.timestamp,
                similarity: sim,
                summary_text: entry.summary_text.clone(),
                regret_score: entry.regret_score,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn save_to_disk(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let json = serde_json::to_string(&self.entries)?;
        std::fs::write(&self.db_path, json)?;
        Ok(())
    }

    fn load_from_disk(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if std::path::Path::new(&self.db_path).exists() {
            let json = std::fs::read_to_string(&self.db_path)?;
            let entries: HashMap<String, VectorEntry> = serde_json::from_str(&json)?;
            self.entries = entries;
            println!(
                "[VectorMemory] ✅ Loaded {} entries from disk",
                self.entries.len()
            );
        }
        Ok(())
    }
}

/// Compute cosine similarity between two f32 vectors.
/// Both vectors should be L2-normalized (which Ollama returns).
/// Range: -1.0 to 1.0, clamped to 0.0 to 1.0 for non-negative similarity.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    dot.clamp(0.0, 1.0) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![0.5, 0.5, 0.5, 0.5];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_partial() {
        let a = vec![1.0, 0.0];
        let b = vec![0.5, 0.5];
        let sim = cosine_similarity(&a, &b);
        // dot product of [1,0] * [0.5,0.5] = 0.5
        assert!((sim - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_search_by_vector_empty_store() {
        let mem = VectorMemory::new(":memory:");
        let results = mem.search_by_vector(&[1.0, 0.0], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_by_vector_with_data() {
        let mut mem = VectorMemory::new(":memory:");
        // Manually insert entries (no Ollama call needed)
        mem.entries.insert(
            "ep1".to_string(),
            VectorEntry {
                episode_id: "ep1".to_string(),
                symbol: "BTC".to_string(),
                timestamp: Utc::now(),
                embedding: vec![1.0, 0.0],
                summary_text: "Bullish Bitcoin trade".to_string(),
                regret_score: None,
            },
        );
        mem.entries.insert(
            "ep2".to_string(),
            VectorEntry {
                episode_id: "ep2".to_string(),
                symbol: "NIFTY".to_string(),
                timestamp: Utc::now(),
                embedding: vec![0.0, 1.0],
                summary_text: "NIFTY range trade".to_string(),
                regret_score: None,
            },
        );

        // Search for something similar to ep1
        let results = mem.search_by_vector(&[0.9, 0.1], 2);
        assert_eq!(results.len(), 2);
        // ep1 should be first (higher similarity)
        assert_eq!(results[0].episode_id, "ep1");
        assert!(results[0].similarity > 0.8);
    }
}
