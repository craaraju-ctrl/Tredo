use crate::LlmExecutor;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── LanceDB backend (feature-gated) ─────────────────────────────────────────
#[cfg(feature = "lancedb")]
mod lance_backend {
    use super::*;
    use arrow_array::{
        Array, Float32Array, Float64Array, RecordBatch, RecordBatchIterator, StringArray,
    };
    use arrow_schema::{DataType, Field, Schema};
    use chrono::{DateTime, Utc};
    use futures::TryStreamExt;
    use lancedb::connect;
    use lancedb::query::ExecutableQuery;
    use lancedb::query::QueryBase;
    use std::sync::Arc;

    /// The LanceDbBackend wraps the table, created lazily on first store.
    pub struct LanceDbBackend {
        table: lancedb::Table,
    }

    /// The LanceDbBackend holds the connection and the table ready for operations.
    impl LanceDbBackend {
        /// Attempt to open the existing table or create a new empty one.
        /// `db_path` is treated as a directory (LanceDB stores a dataset).
        pub async fn open_or_create(
            db_path: &str,
            dim: usize,
        ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            let conn = connect(db_path).execute().await?;

            // Try opening an existing table first, otherwise create empty
            let table = match conn.open_table("episodes").execute().await {
                Ok(tbl) => {
                    println!("[VectorMemory] 🔍 Reopened existing LanceDB table 'episodes'");
                    tbl
                }
                Err(_) => {
                    let schema = Arc::new(Schema::new(vec![
                        Field::new("episode_id", DataType::Utf8, false),
                        Field::new("symbol", DataType::Utf8, false),
                        Field::new("timestamp", DataType::Utf8, false),
                        Field::new("summary_text", DataType::Utf8, false),
                        Field::new("regret_score", DataType::Float64, true),
                        Field::new(
                            "embedding",
                            DataType::FixedSizeList(
                                Arc::new(Field::new("item", DataType::Float32, false)),
                                dim as i32,
                            ),
                            false,
                        ),
                    ]));
                    let tbl = conn
                        .create_empty_table("episodes", schema)
                        .execute()
                        .await?;
                    println!(
                        "[VectorMemory] ✅ Created new LanceDB table 'episodes' (dim={})",
                        dim
                    );
                    tbl
                }
            };

            Ok(Self { table })
        }

        /// Store a single vector entry as a RecordBatch.
        pub async fn store(
            &mut self,
            entry: &VectorEntry,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let dim = entry.embedding.len() as i32;

            let episode_ids = StringArray::from(vec![entry.episode_id.as_str()]);
            let symbols = StringArray::from(vec![entry.symbol.as_str()]);
            let timestamps = StringArray::from(vec![entry.timestamp.to_rfc3339().as_str()]);
            let summaries = StringArray::from(vec![entry.summary_text.as_str()]);

            let regret_values: Vec<f64> = match entry.regret_score {
                Some(v) => vec![v],
                None => vec![f64::NAN],
            };
            let regrets = Float64Array::from(regret_values);

            let embedding_values = Float32Array::from(entry.embedding.clone());
            let embedding_array = Arc::new(arrow_array::FixedSizeListArray::new(
                Arc::new(Field::new("item", DataType::Float32, false)),
                dim,
                Arc::new(embedding_values),
                None,
            ));

            let table_schema = self.table.schema().await?;
            let batch = RecordBatch::try_new(
                table_schema.clone(),
                vec![
                    Arc::new(episode_ids),
                    Arc::new(symbols),
                    Arc::new(timestamps),
                    Arc::new(summaries),
                    Arc::new(regrets),
                    embedding_array,
                ],
            )?;

            // Wrap the batch in a RecordBatchIterator (which implements RecordBatchReader)
            // so it satisfies lancedb's IntoArrow trait bound.
            let reader = RecordBatchIterator::new(vec![batch].into_iter().map(Ok), table_schema);
            self.table.add(reader).execute().await?;
            Ok(())
        }

        /// Search for similar vectors with optional metadata filtering.
        pub async fn search(
            &self,
            query_embedding: &[f32],
            top_k: usize,
            symbol_filter: Option<&str>,
            max_regret: Option<f64>,
        ) -> Result<Vec<SimilarResult>, Box<dyn std::error::Error + Send + Sync>> {
            let mut filter = String::new();
            if let Some(sym) = symbol_filter {
                filter.push_str(&format!("symbol = '{}'", sym));
            }
            if let Some(reg) = max_regret {
                if !filter.is_empty() {
                    filter.push_str(" AND ");
                }
                filter.push_str(&format!("regret_score <= {}", reg));
            }

            let mut q = self.table.query().nearest_to(query_embedding)?.limit(top_k);

            if !filter.is_empty() {
                q = q.only_if(&filter);
            }

            let stream = q.execute().await?;
            let batches: Vec<RecordBatch> = stream.try_collect().await?;

            let mut results = Vec::new();
            for batch in &batches {
                let episode_ids = batch
                    .column_by_name("episode_id")
                    .ok_or("missing episode_id column")?
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or("episode_id not StringArray")?;

                let symbols = batch
                    .column_by_name("symbol")
                    .ok_or("missing symbol column")?
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or("symbol not StringArray")?;

                let timestamps = batch
                    .column_by_name("timestamp")
                    .ok_or("missing timestamp column")?
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or("timestamp not StringArray")?;

                let summaries = batch
                    .column_by_name("summary_text")
                    .ok_or("missing summary_text column")?
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or("summary_text not StringArray")?;

                let regret_scores = batch
                    .column_by_name("regret_score")
                    .ok_or("missing regret_score column")?
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or("regret_score not Float64Array")?;

                // LanceDB adds a distance column with the similarity score
                // (column name is `_distance` in recent LanceDB versions;
                // fall back to `distance` for compatibility).
                let dist_col = batch
                    .column_by_name("_distance")
                    .or_else(|| batch.column_by_name("distance"))
                    .ok_or("missing distance column (tried _distance, distance)")?;
                let distances = dist_col
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or("distance column not Float64Array")?;

                for i in 0..batch.num_rows() {
                    let ts_str = timestamps.value(i);
                    let ts: DateTime<Utc> = match ts_str.parse() {
                        Ok(t) => t,
                        Err(_) => {
                            eprintln!("[VectorMemory] ⚠️ Failed to parse timestamp: {}", ts_str);
                            continue;
                        }
                    };

                    let regret: Option<f64> = if regret_scores.is_null(i) {
                        None
                    } else {
                        let v = regret_scores.value(i);
                        if v.is_nan() {
                            None
                        } else {
                            Some(v)
                        }
                    };

                    // LanceDB returns distance (lower = closer). The default metric is L2.
                    // Convert to similarity: similarity = 1.0 / (1.0 + distance)
                    // Note: this formula assumes L2 distance. If cosine distance was configured
                    // (_distance = 1.0 - cos_sim), adjust to: similarity = 1.0 - dist.
                    let dist = distances.value(i);
                    let similarity = 1.0 / (1.0 + dist);

                    results.push(SimilarResult {
                        episode_id: episode_ids.value(i).to_string(),
                        symbol: symbols.value(i).to_string(),
                        timestamp: ts,
                        similarity,
                        summary_text: summaries.value(i).to_string(),
                        regret_score: regret,
                    });
                }
            }

            Ok(results)
        }
    }
}

#[cfg(feature = "lancedb")]
use lance_backend::LanceDbBackend;

// ── Embedding Generator Trait ───────────────────────────────────────────────

/// Trait for generating embeddings from text. Implemented by LlmExecutor.
pub trait EmbeddingGenerator {
    fn embed_text_blocking(&self, text: &str) -> Vec<f32>;
}

// ── EpisodicVectorRecord ────────────────────────────────────────────────────

/// A lightweight record for vectorized episodic memory with versioned context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicVectorRecord {
    pub episode_id: String,
    pub embedding: Vec<f32>,
    pub epoch_timestamp: u64,
    pub net_pnl_pct: f64,
    pub trade_direction: String,
    pub rule_version: u32,
    pub context_json: String,
}

/// LanceMemoryStore — an in-memory store of EpisodicVectorRecord with
/// serialization helpers for version-tagged context strings.
///
/// Generic over `E: EmbeddingGenerator` so the embedding mechanism can be
/// injected (e.g. LlmExecutor or a mock for testing).
#[derive(Debug, Clone)]
pub struct LanceMemoryStore<E: EmbeddingGenerator> {
    pub records: Vec<EpisodicVectorRecord>,
    pub dimension: usize,
    pub embedder: E,
}

impl<E: EmbeddingGenerator> LanceMemoryStore<E> {
    pub fn new(dimension: usize, embedder: E) -> Self {
        Self {
            records: Vec::new(),
            dimension,
            embedder,
        }
    }

    /// Serializes context while binding the operational version directly to the payload string.
    pub fn serialize_context_with_version(
        &self,
        snapshot: &std::collections::HashMap<String, f64>,
        rule_version: u32,
    ) -> String {
        let mut metrics: Vec<String> = snapshot
            .iter()
            .map(|(s, v)| format!("{}:{:.2}", s, v))
            .collect();
        metrics.sort();
        format!("v{} | Context: [{}]", rule_version, metrics.join(", "))
    }

    pub fn push(&mut self, record: EpisodicVectorRecord) {
        self.records.push(record);
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

// ── Data Types ──────────────────────────────────────────────────

/// A stored vector entry with metadata for similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    pub episode_id: String,
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub embedding: Vec<f32>,
    pub summary_text: String,
    pub regret_score: Option<f64>,
}

/// Result of a similarity search.
#[derive(Debug, Clone)]
pub struct SimilarResult {
    pub episode_id: String,
    pub symbol: String,
    pub timestamp: DateTime<Utc>,
    pub similarity: f64,
    pub summary_text: String,
    pub regret_score: Option<f64>,
}

// ── JSON Fallback Backend (always available) ──────────────────────────────

#[derive(Debug)]
struct JsonBackend {
    entries: HashMap<String, VectorEntry>,
    db_path: String,
}

impl JsonBackend {
    fn new(db_path: &str) -> Self {
        let mut b = Self {
            entries: HashMap::new(),
            db_path: db_path.to_string(),
        };
        let _ = b.load_from_disk();
        b
    }

    fn store(&mut self, entry: VectorEntry) {
        self.entries.insert(entry.episode_id.clone(), entry);
        let _ = self.save_to_disk();
    }

    fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<SimilarResult> {
        if self.entries.is_empty() {
            return vec![];
        }

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

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
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
                "[VectorMemory] ✅ Loaded {} entries from JSON disk",
                self.entries.len()
            );
        }
        Ok(())
    }

    /// Derive a LanceDB directory path from the JSON db_path.
    /// e.g. "tredo_vectors.json" => "tredo_vectors.lance"
    #[cfg(feature = "lancedb")]
    fn lance_db_path(&self) -> String {
        let p = std::path::Path::new(&self.db_path);
        match p.extension() {
            Some(ext) if ext == "json" => {
                let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                format!("{}.lance", stem)
            }
            _ => format!("{}.lance", self.db_path),
        }
    }
}

// ── Public VectorMemory (identical API, LanceDB-ready) ──────────────────────

/// Production-ready VectorMemory with optional LanceDB backend.
///
/// - **Default** (no features): JSON file + brute-force cosine similarity.
/// - **With `lancedb` feature**: `cargo build -p tredo-core --features lancedb`
///   Enables LanceDB embedded vector DB with ANN indexing and metadata filtering.
///
/// LanceDB is initialized lazily on the first `store()` call (to keep the
/// constructor synchronous). If LanceDB init fails, it falls back to JSON.
#[derive(Debug)]
pub struct VectorMemory {
    #[cfg(feature = "lancedb")]
    lancedb: Option<LanceDbBackend>,
    json: JsonBackend,
}

impl VectorMemory {
    /// Create a new VectorMemory. Always synchronous.
    ///
    /// - `db_path`: Path to the JSON file (e.g. `"tredo_vectors.json"`).
    ///   When the `lancedb` feature is enabled, LanceDB will create a sibling
    ///   directory at `{stem}.lance` (e.g. `"tredo_vectors.lance/"`).
    pub fn new(db_path: &str) -> Self {
        println!("[VectorMemory] Initialized (JSON backend: {})", db_path);

        Self {
            #[cfg(feature = "lancedb")]
            lancedb: None,
            json: JsonBackend::new(db_path),
        }
    }

    /// Store a new vector entry, embedding the text via the LLM executor.
    ///
    /// When the `lancedb` feature is enabled, this lazily initializes the
    /// LanceDB backend on the first call (migrating any existing JSON data).
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

        #[cfg(feature = "lancedb")]
        {
            if self.lancedb.is_none() {
                let lance_path = self.json.lance_db_path();
                let dim = entry.embedding.len();
                match LanceDbBackend::open_or_create(&lance_path, dim).await {
                    Ok(mut backend) => {
                        println!(
                            "[VectorMemory] ✅ LanceDB backend initialized at {}",
                            lance_path
                        );

                        // Migrate existing JSON entries to LanceDB on first init
                        let existing: Vec<VectorEntry> =
                            self.json.entries.values().cloned().collect();
                        for old_entry in &existing {
                            if let Err(e) = backend.store(old_entry).await {
                                eprintln!(
                                    "[VectorMemory] ⚠️ Failed to migrate entry {}: {}",
                                    old_entry.episode_id, e
                                );
                            }
                        }
                        if !existing.is_empty() {
                            println!(
                                "[VectorMemory] 📦 Migrated {} entries from JSON to LanceDB",
                                existing.len()
                            );
                        }

                        self.lancedb = Some(backend);
                    }
                    Err(e) => {
                        eprintln!(
                            "[VectorMemory] ⚠️ LanceDB init failed: {}. Falling back to JSON.",
                            e
                        );
                    }
                }
            }

            if let Some(lance) = &mut self.lancedb {
                return lance.store(&entry).await;
            }
        }

        // JSON fallback path
        self.json.store(entry);
        Ok(())
    }

    /// Search for the `top_k` most similar entries by embedding the query text.
    ///
    /// When LanceDB is active, uses ANN search with proper vector indexing.
    /// Otherwise, falls back to brute-force cosine similarity on JSON data.
    pub async fn search(
        &self,
        query_text: &str,
        top_k: usize,
        llm: &LlmExecutor,
    ) -> Result<Vec<SimilarResult>, Box<dyn std::error::Error + Send + Sync>> {
        if self.is_empty() {
            return Ok(vec![]);
        }

        let query_embedding = llm.embed_text(query_text).await?;

        #[cfg(feature = "lancedb")]
        if let Some(lance) = &self.lancedb {
            return lance.search(&query_embedding, top_k, None, None).await;
        }

        // JSON brute-force fallback (also used when lancedb feature is off)
        Ok(self.json.search(&query_embedding, top_k))
    }

    /// Synchronous search by raw vector. Always uses the JSON backend.
    ///
    /// For performance-sensitive paths (LLM embedding already available),
    /// prefer the async [`search`](Self::search) method which uses LanceDB's
    /// ANN index when available.
    pub fn search_by_vector(&self, query_embedding: &[f32], top_k: usize) -> Vec<SimilarResult> {
        self.json.search(query_embedding, top_k)
    }

    pub fn len(&self) -> usize {
        self.json.len()
    }

    pub fn is_empty(&self) -> bool {
        self.json.is_empty()
    }
}

// ── Cosine Similarity (unchanged) ──────────────────────────────────────────

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

// ── Tests ───────────────────────────────────────────────────────────────────

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
        mem.json.entries.insert(
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
        mem.json.entries.insert(
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

        let results = mem.search_by_vector(&[0.9, 0.1], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].episode_id, "ep1");
        assert!(results[0].similarity > 0.8);
    }

    #[test]
    fn test_json_roundtrip() {
        let path = "_test_vectors.json";
        // Clean up from previous runs
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all("_test_vectors.lance");

        // Use L2-normalized vectors so identical vectors give dot product = 1.0.
        // [0.6, 0.8] has L2 norm = 1.0.
        let embedding = vec![0.6_f32, 0.8_f32];

        {
            let mut mem = VectorMemory::new(path);
            mem.json.entries.insert(
                "ep_rt1".to_string(),
                VectorEntry {
                    episode_id: "ep_rt1".to_string(),
                    symbol: "ETH".to_string(),
                    timestamp: Utc::now(),
                    embedding: embedding.clone(),
                    summary_text: "Ethereum breakout".to_string(),
                    regret_score: Some(0.2),
                },
            );
            mem.json.save_to_disk().unwrap();
        }

        // Re-open and verify
        {
            let mut mem = VectorMemory::new(path);
            // Force reload (automatically done in new())
            let _ = mem.json.load_from_disk();
            assert_eq!(mem.len(), 1);
            let results = mem.search_by_vector(&embedding, 5);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].episode_id, "ep_rt1");
            assert!((results[0].similarity - 1.0).abs() < 0.001);
        }

        // Cleanup
        let _ = std::fs::remove_file(path);
    }
}
