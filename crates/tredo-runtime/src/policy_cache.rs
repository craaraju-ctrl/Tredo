//! # Policy Cache — Learned Trading Memory
//!
//! Records (features → action → outcome) tuples for every trade and uses
//! them to short-circuit the expensive 5-call Ollama debate when the system
//! has high confidence in a cached decision.
//!
//! ## How it works
//! 1. **Extract features** — Bucket market state (regime, RSI, confluence, etc.)
//! 2. **Hash lookup** — Find matching entries in the cache
//! 3. **Confidence check** — Enough samples? Win rate high enough? → use cache
//! 4. **Fallback** — Low confidence → call Ollama (existing pipeline)
//! 5. **Record** — After trade closes, update the cache with the outcome
//!
//! This is your "pre-trained trading memory" — it's a learned lookup table,
//! not a neural network. It's interpretable, self-improving, and honest
//! about when it doesn't have enough data.

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tredo_autonomous::state::SharedState;
use tredo_autonomous::types::MarketRegime;
use tredo_core::TradeDirection;

/// Bucketed market features used as the cache key.
///
/// Features are deliberately coarse-bucketed so that similar market
/// conditions map to the same cache entry. This accelerates learning
/// — a setup seen at 9:32 AM is also matched at 9:45 AM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketFeatures {
    pub symbol: String,
    pub regime: String,
    /// Confluence score floored to 0-9 (0.0-0.1 → 0, 0.9-1.0 → 9)
    pub confluence_bucket: u8,
    /// RSI floored to 0-9 (0-10 → 0, 90-100 → 9)
    pub rsi_bucket: u8,
    /// Trend direction: -1 (bear), 0 (neutral), +1 (bull)
    pub trend_bucket: i8,
    /// ATR % floored to 0-9 (0-1% → 0, 9-10% → 9)
    pub volatility_bucket: u8,
    /// Hour of day (0-23)
    pub time_of_day: u8,
    /// Day of week (0=Mon, 6=Sun)
    pub day_of_week: u8,
}

impl Hash for MarketFeatures {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.symbol.hash(state);
        self.regime.hash(state);
        self.confluence_bucket.hash(state);
        self.rsi_bucket.hash(state);
        self.trend_bucket.hash(state);
        self.volatility_bucket.hash(state);
        self.time_of_day.hash(state);
        // Deliberately NOT hashing day_of_week — patterns transfer across days
    }
}

/// A single entry in the policy cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEntry {
    pub features: MarketFeatures,
    pub recommended_action: TradeDirection,
    pub sample_size: u32,
    pub wins: u32,
    pub losses: u32,
    pub avg_pnl_pct: f64,
    pub avg_regret: f64,
    pub last_updated: DateTime<Utc>,
    /// How many of the samples were originally Ollama-derived
    pub ollama_decisions: u32,
}

impl PolicyEntry {
    pub fn win_rate(&self) -> f64 {
        if self.sample_size == 0 {
            return 0.5;
        }
        self.wins as f64 / self.sample_size as f64
    }

    /// Bayesian-ish confidence score.
    ///
    /// Factor 1: Sample size (0 at 0 samples, 1 at 30+ samples)
    /// Factor 2: Win rate distance from 50% (0 at 50%, 1 at 0% or 100%)
    pub fn confidence(&self) -> f64 {
        let size_factor = (self.sample_size as f64 / 30.0).min(1.0);
        let rate_factor = (self.win_rate() - 0.5).abs() * 2.0; // 0 at 50%, 1 at 0%/100%
        (size_factor * 0.6 + rate_factor * 0.4).clamp(0.0, 1.0)
    }
}

/// Configuration for the policy cache's decision thresholds.
#[derive(Debug, Clone)]
pub struct PolicyCacheConfig {
    /// Minimum samples before trusting the cache
    pub min_samples: u32,
    /// Minimum win rate to recommend a trade (0.55 = 55%)
    pub min_win_rate: f64,
    /// Minimum confidence score to override Ollama
    pub min_confidence: f64,
}

impl Default for PolicyCacheConfig {
    fn default() -> Self {
        Self {
            min_samples: 5,
            min_win_rate: 0.55,
            min_confidence: 0.6,
        }
    }
}

/// Maximum number of hit-rate snapshots kept for the sparkline trend.
const MAX_HIT_RATE_HISTORY: usize = 60;

/// Owned wrapper struct for loading from disk — owns the entries HashMap.
#[derive(Deserialize)]
struct PolicyCacheDisk {
    entries: std::collections::HashMap<u64, PolicyEntry>,
    total_lookups: u64,
    total_hits: u64,
    /// Hit-rate history for sparkline trend (sampled every save, max ~60 entries = 30 min).
    #[serde(default)]
    hit_rate_history: Vec<f64>,
    /// Top-performers average win rate history for sparkline trend.
    #[serde(default)]
    top_win_rate_history: Vec<f64>,
    /// Cumulative P&L history for dashboard sparkline trend.
    #[serde(default)]
    pnl_history: Vec<f64>,
    /// Equity curve history for dashboard sparkline trend.
    #[serde(default)]
    equity_history: Vec<f64>,
    /// Top-performers average confidence history for sparkline trend.
    #[serde(default)]
    confidence_history: Vec<f64>,
    /// Global consecutive win/loss streak history for sparkline trend.
    #[serde(default)]
    streak_history: Vec<f64>,
}

/// Borrowed wrapper struct for saving to disk — borrows from the read guard.
/// Avoids cloning the entire HashMap before serialization.
#[derive(Serialize)]
struct PolicyCacheDiskRef<'a> {
    entries: &'a std::collections::HashMap<u64, PolicyEntry>,
    total_lookups: u64,
    total_hits: u64,
    /// Hit-rate history for sparkline trend.
    hit_rate_history: &'a [f64],
    /// Top-performers average win rate history for sparkline trend.
    top_win_rate_history: &'a [f64],
    /// Cumulative P&L history for dashboard sparkline trend.
    pnl_history: &'a [f64],
    /// Equity curve history for dashboard sparkline trend.
    equity_history: &'a [f64],
    /// Top-performers average confidence history for sparkline trend.
    confidence_history: &'a [f64],
    /// Global consecutive win/loss streak history for sparkline trend.
    streak_history: &'a [f64],
}

/// Core policy cache — the "learned trading memory" of the agent.
///
/// Thread-safe via `parking_lot::RwLock`. Persisted to `~/.tredo/policy_cache.json`.
/// Also tracks runtime hit/miss statistics for monitoring cache effectiveness.
/// Hit/miss counters are persisted alongside the cache entries on every save().
pub struct PolicyCache {
    state: SharedState,
    config: PolicyCacheConfig,
    cache: parking_lot::RwLock<HashMap<u64, PolicyEntry>>,
    /// Total number of cache lookup attempts (every call to `make_decision`)
    cache_lookups: std::sync::atomic::AtomicU64,
    /// Number of lookups that resulted in a cache hit (skipped Ollama)
    cache_hits: std::sync::atomic::AtomicU64,
    /// Rolling history of hit-rate snapshots (sampled on every save) for sparkline trend.
    hit_rate_history: parking_lot::RwLock<Vec<f64>>,
    /// Rolling history of top-performers average win rate (sampled on every save) for sparkline trend.
    top_win_rate_history: parking_lot::RwLock<Vec<f64>>,
    /// Rolling history of cumulative P&L snapshots (sampled on every save) for dashboard sparkline trend.
    pnl_history: parking_lot::RwLock<Vec<f64>>,
    /// Rolling history of equity curve snapshots (sampled on every save) for dashboard sparkline trend.
    equity_history: parking_lot::RwLock<Vec<f64>>,
    /// Rolling history of top-performers average confidence (sampled on every save) for sparkline trend.
    confidence_history: parking_lot::RwLock<Vec<f64>>,
    /// Global consecutive win/loss streak. Positive = winning streak, negative = losing streak.
    current_streak: std::sync::atomic::AtomicI32,
    /// Rolling history of consecutive win/loss streak snapshots for sparkline trend.
    streak_history: parking_lot::RwLock<Vec<f64>>,
}

impl PolicyCache {
    /// Create a new empty cache.
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            config: PolicyCacheConfig::default(),
            cache: parking_lot::RwLock::new(HashMap::new()),
            cache_lookups: std::sync::atomic::AtomicU64::new(0),
            cache_hits: std::sync::atomic::AtomicU64::new(0),
            hit_rate_history: parking_lot::RwLock::new(Vec::with_capacity(MAX_HIT_RATE_HISTORY)),
            top_win_rate_history: parking_lot::RwLock::new(Vec::with_capacity(
                MAX_HIT_RATE_HISTORY,
            )),
            pnl_history: parking_lot::RwLock::new(Vec::with_capacity(MAX_HIT_RATE_HISTORY)),
            equity_history: parking_lot::RwLock::new(Vec::with_capacity(MAX_HIT_RATE_HISTORY)),
            confidence_history: parking_lot::RwLock::new(Vec::with_capacity(MAX_HIT_RATE_HISTORY)),
            current_streak: std::sync::atomic::AtomicI32::new(0),
            streak_history: parking_lot::RwLock::new(Vec::with_capacity(MAX_HIT_RATE_HISTORY)),
        }
    }

    /// Create a cache and immediately load from disk.
    pub fn from_disk(state: SharedState) -> Self {
        let cache = Self::new(state);
        cache.load();
        cache
    }

    /// Number of unique entries in the cache.
    pub fn size(&self) -> usize {
        self.cache.read().len()
    }

    /// Total samples across all entries.
    pub fn total_samples(&self) -> u32 {
        self.cache.read().values().map(|e| e.sample_size).sum()
    }

    /// Get the current config.
    pub fn config(&self) -> &PolicyCacheConfig {
        &self.config
    }

    /// Set new config thresholds.
    pub fn set_config(&mut self, config: PolicyCacheConfig) {
        self.config = config;
    }

    /// Extract current market features for a symbol.
    ///
    /// This is the "feature engineering" step — it reads live state and
    /// buckets it into coarsely-grained categories.
    pub async fn extract_features(&self, symbol: &str) -> MarketFeatures {
        let history = self.state.ohlcv_history.read().await;
        let regime = format!(
            "{:?}",
            self.state
                .market_regime
                .read()
                .await
                .unwrap_or(MarketRegime::Ranging)
        );
        let bars = history.get(symbol).cloned();
        drop(history);

        let (rsi, confluence, volatility_pct) = match bars {
            Some(ref b) if b.len() >= 14 => {
                let rsi = tredo_autonomous::helpers::compute_rsi(b, 14);
                let last = b.last().map(|x| x.close).unwrap_or(0.0);
                let conf = compute_confluence_simple(symbol, last, b);
                let vol = helpers::compute_atr(b, 14) / last.max(0.001);
                (rsi, conf, vol)
            }
            _ => (50.0, 0.5, 0.01),
        };

        let now = Utc::now();

        MarketFeatures {
            symbol: symbol.to_string(),
            regime,
            confluence_bucket: (confluence * 10.0).floor().clamp(0.0, 9.0) as u8,
            rsi_bucket: (rsi / 10.0).floor().clamp(0.0, 9.0) as u8,
            trend_bucket: 0, // TODO: read from RegimeDetector when available
            volatility_bucket: (volatility_pct * 100.0).floor().clamp(0.0, 9.0) as u8,
            time_of_day: now.hour() as u8,
            day_of_week: now.weekday().num_days_from_monday() as u8,
        }
    }

    /// Look up a cached policy for the given features.
    ///
    /// Returns `Some(entry)` if:
    /// - We have enough samples (`>= min_samples`)
    /// - Win rate is above threshold (`>= min_win_rate`)
    /// - Confidence is above threshold (`>= min_confidence`)
    ///
    /// Returns `None` if any condition fails (= fall back to Ollama).
    pub fn lookup(&self, features: &MarketFeatures) -> Option<PolicyEntry> {
        let hash = Self::hash_features(features);
        let entry = self.cache.read().get(&hash).cloned()?;

        if entry.sample_size < self.config.min_samples {
            return None;
        }
        if entry.win_rate() < self.config.min_win_rate {
            return None;
        }
        if entry.confidence() < self.config.min_confidence {
            return None;
        }

        Some(entry)
    }

    /// Record the outcome of a trade (called after position closes).
    ///
    /// Updates running statistics: win count, loss count, average P&L%, regret.
    pub fn record_outcome(
        &self,
        features: &MarketFeatures,
        action: TradeDirection,
        profitable: bool,
        pnl_pct: f64,
        regret: f64,
        from_ollama: bool,
    ) {
        // Update global streak before taking the write lock
        use std::sync::atomic::Ordering;
        if profitable {
            let cur = self.current_streak.load(Ordering::Relaxed);
            if cur > 0 {
                self.current_streak.fetch_add(1, Ordering::Relaxed);
            } else {
                self.current_streak.store(1, Ordering::Relaxed);
            }
        } else {
            let cur = self.current_streak.load(Ordering::Relaxed);
            if cur < 0 {
                self.current_streak.fetch_sub(1, Ordering::Relaxed);
            } else {
                self.current_streak.store(-1, Ordering::Relaxed);
            }
        }

        let hash = Self::hash_features(features);
        let mut cache = self.cache.write();
        let entry = cache.entry(hash).or_insert_with(|| PolicyEntry {
            features: features.clone(),
            recommended_action: action,
            sample_size: 0,
            wins: 0,
            losses: 0,
            avg_pnl_pct: 0.0,
            avg_regret: 0.0,
            last_updated: Utc::now(),
            ollama_decisions: 0,
        });

        let n = entry.sample_size as f64;
        entry.sample_size += 1;
        if profitable {
            entry.wins += 1;
        } else {
            entry.losses += 1;
        }
        entry.avg_pnl_pct = (entry.avg_pnl_pct * n + pnl_pct) / (n + 1.0);
        entry.avg_regret = (entry.avg_regret * n + regret) / (n + 1.0);
        if from_ollama {
            entry.ollama_decisions += 1;
        }
        entry.last_updated = Utc::now();
    }

    /// Seed the cache from historical closed trades in the episode store.
    ///
    /// Called once at startup to bootstrap the cache from past experience.
    pub async fn seed_from_history(&self) -> usize {
        let store = &self.state.episode_store;
        let trades = match store.load_recent_closed_trades(500, None) {
            Ok(t) => t,
            Err(_) => return 0,
        };

        let mut seeded = 0;
        for trade in &trades {
            if trade.entry_time.is_empty() {
                continue;
            }

            let entry_time = chrono::DateTime::parse_from_rfc3339(&trade.entry_time)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let features = MarketFeatures {
                symbol: trade.symbol.clone(),
                regime: trade.market_regime.clone(),
                confluence_bucket: (trade.confluence_score * 10.0).floor().clamp(0.0, 9.0) as u8,
                rsi_bucket: 5, // unknown historically, use neutral
                trend_bucket: 0,
                volatility_bucket: 5,
                time_of_day: entry_time.hour() as u8,
                day_of_week: entry_time.weekday().num_days_from_monday() as u8,
            };

            let pnl_pct = trade.pnl_pct;
            let profitable = trade.was_correct;
            let regret = trade.regret_score;
            let action = match trade.direction.as_str() {
                "Long" => TradeDirection::Long,
                "Short" => TradeDirection::Short,
                _ => continue,
            };

            self.record_outcome(&features, action, profitable, pnl_pct, regret, true);
            seeded += 1;
        }

        tracing::info!("PolicyCache seeded with {} historical trades", seeded);
        seeded
    }

    /// Persist the cache to disk (JSON at `~/.tredo/policy_cache.json`).
    /// Saves entries plus hit/miss counters so the orchestrator can read them.
    ///
    /// Also records hit-rate and top-performer win-rate snapshots for sparkline trends.
    ///
    /// Uses a borrowed wrapper struct to avoid cloning the entire HashMap
    /// before serialization. The read lock is held during serialization but
    /// released before the blocking file write.
    pub fn save(&self) {
        // Record snapshots for sparkline trends
        self.record_hit_rate_snapshot();
        self.record_top_win_rate_snapshot();
        self.record_confidence_snapshot();
        self.record_streak_snapshot();

        let entries = self.cache.read();
        let total_lookups = self
            .cache_lookups
            .load(std::sync::atomic::Ordering::Relaxed);
        let total_hits = self.cache_hits.load(std::sync::atomic::Ordering::Relaxed);

        let hit_history = self.hit_rate_history.read();
        let wr_history = self.top_win_rate_history.read();
        let pnl_hist = self.pnl_history.read();
        let eq_hist = self.equity_history.read();
        let conf_hist = self.confidence_history.read();
        let streak_hist = self.streak_history.read();
        let disk = PolicyCacheDiskRef {
            entries: &entries,
            total_lookups,
            total_hits,
            hit_rate_history: &hit_history,
            top_win_rate_history: &wr_history,
            pnl_history: &pnl_hist,
            equity_history: &eq_hist,
            confidence_history: &conf_hist,
            streak_history: &streak_hist,
        };

        let json = match serde_json::to_string(&disk) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to serialize policy cache: {}", e);
                return;
            }
        };
        drop(entries);
        drop(hit_history);
        drop(wr_history);
        drop(pnl_hist);
        drop(eq_hist);
        drop(conf_hist);
        drop(streak_hist);

        let path = Self::cache_path();
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!("Failed to save policy cache: {}", e);
        }
    }

    /// Load the cache from disk.
    /// Restores entries AND cached hit/miss counters from the previous run.
    pub fn load(&self) {
        let path = Self::cache_path();
        if !path.exists() {
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<PolicyCacheDisk>(&json) {
                Ok(disk) => {
                    let n = disk.entries.len();
                    let hr_len = disk.hit_rate_history.len();
                    let wr_len = disk.top_win_rate_history.len();
                    let pnl_len = disk.pnl_history.len();
                    let eq_len = disk.equity_history.len();
                    let conf_len = disk.confidence_history.len();
                    let streak_len = disk.streak_history.len();
                    *self.cache.write() = disk.entries;
                    self.cache_lookups
                        .store(disk.total_lookups, std::sync::atomic::Ordering::Relaxed);
                    self.cache_hits
                        .store(disk.total_hits, std::sync::atomic::Ordering::Relaxed);
                    *self.hit_rate_history.write() = disk.hit_rate_history;
                    *self.top_win_rate_history.write() = disk.top_win_rate_history;
                    *self.pnl_history.write() = disk.pnl_history;
                    *self.equity_history.write() = disk.equity_history;
                    *self.confidence_history.write() = disk.confidence_history;
                    *self.streak_history.write() = disk.streak_history;
                    tracing::info!(
                        "PolicyCache loaded {} entries from disk (lookups={}, hits={}, hr={}, wr={}, pnl={}, eq={}, conf={}, streak={})",
                        n, disk.total_lookups, disk.total_hits,
                        hr_len, wr_len, pnl_len, eq_len, conf_len, streak_len
                    );
                }
                Err(_) => {
                    // Fallback: try loading as legacy HashMap<u64, PolicyEntry> (pre-counter format)
                    if let Ok(loaded) = serde_json::from_str::<HashMap<u64, PolicyEntry>>(&json) {
                        let n = loaded.len();
                        *self.cache.write() = loaded;
                        tracing::info!(
                            "PolicyCache loaded {} entries from disk (legacy format)",
                            n
                        );
                    } else {
                        tracing::warn!("Failed to parse policy cache (tried both formats)");
                    }
                }
            },
            Err(e) => tracing::warn!("Failed to read policy cache: {}", e),
        }
    }

    /// Get all entries (for inspection / debugging).
    pub fn all_entries(&self) -> Vec<PolicyEntry> {
        self.cache.read().values().cloned().collect()
    }

    /// Record a cache lookup attempt.
    ///
    /// Call this after every decision: `was_hit=true` if the cache provided
    /// the decision (hit), `false` if it fell through to Ollama (miss).
    pub fn record_lookup(&self, was_hit: bool) {
        self.cache_lookups
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if was_hit {
            self.cache_hits
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Get current hit/miss statistics.
    ///
    /// Returns `(total_lookups, cache_hits, hit_rate)` where hit_rate is 0.0-1.0.
    pub fn hit_stats(&self) -> (u64, u64, f64) {
        let total = self
            .cache_lookups
            .load(std::sync::atomic::Ordering::Relaxed);
        let hits = self.cache_hits.load(std::sync::atomic::Ordering::Relaxed);
        let rate = if total > 0 {
            hits as f64 / total as f64
        } else {
            0.0
        };
        (total, hits, rate)
    }

    /// Record a hit-rate snapshot for the sparkline trend.
    /// Samples the current hit rate and appends it to the rolling history,
    /// trimming to `MAX_HIT_RATE_HISTORY` entries.
    fn record_hit_rate_snapshot(&self) {
        let (_total, _hits, rate) = self.hit_stats();
        let mut history = self.hit_rate_history.write();
        history.push(rate);
        if history.len() > MAX_HIT_RATE_HISTORY {
            history.remove(0);
        }
    }

    /// Get the hit-rate history for sparkline rendering.
    pub fn hit_rate_history(&self) -> Vec<f64> {
        self.hit_rate_history.read().clone()
    }

    /// Record a top-performers win-rate snapshot for the sparkline trend.
    /// Computes the average win rate of the top 10 entries (min 3 samples each).
    fn record_top_win_rate_snapshot(&self) {
        let top = self.top_performers(3, 10);
        let avg_wr = if top.is_empty() {
            0.0
        } else {
            top.iter().map(|e| e.win_rate()).sum::<f64>() / top.len() as f64
        };
        let mut history = self.top_win_rate_history.write();
        history.push(avg_wr);
        if history.len() > MAX_HIT_RATE_HISTORY {
            history.remove(0);
        }
    }

    /// Get the top-performers win-rate history for sparkline rendering.
    pub fn top_win_rate_history(&self) -> Vec<f64> {
        self.top_win_rate_history.read().clone()
    }

    /// Record a cumulative P&L snapshot for the dashboard sparkline trend.
    /// Called from the engine's periodic save task with the current daily P&L.
    pub fn record_pnl_snapshot(&self, pnl: f64) {
        let mut history = self.pnl_history.write();
        history.push(pnl);
        if history.len() > MAX_HIT_RATE_HISTORY {
            history.remove(0);
        }
    }

    /// Get the P&L history for dashboard sparkline rendering.
    pub fn pnl_history(&self) -> Vec<f64> {
        self.pnl_history.read().clone()
    }

    /// Record an equity curve snapshot for the dashboard sparkline trend.
    /// Called from the engine's periodic save task with the current total equity.
    pub fn record_equity_snapshot(&self, equity: f64) {
        let mut history = self.equity_history.write();
        history.push(equity);
        if history.len() > MAX_HIT_RATE_HISTORY {
            history.remove(0);
        }
    }

    /// Get the equity curve history for dashboard sparkline rendering.
    pub fn equity_history(&self) -> Vec<f64> {
        self.equity_history.read().clone()
    }

    /// Record a top-performers average confidence snapshot for the sparkline trend.
    /// Computes the average confidence score of the top 10 entries (min 3 samples each).
    fn record_confidence_snapshot(&self) {
        let top = self.top_performers(3, 10);
        let avg_conf = if top.is_empty() {
            0.0
        } else {
            top.iter().map(|e| e.confidence()).sum::<f64>() / top.len() as f64
        };
        let mut history = self.confidence_history.write();
        history.push(avg_conf);
        if history.len() > MAX_HIT_RATE_HISTORY {
            history.remove(0);
        }
    }

    /// Get the top-performers average confidence history for sparkline rendering.
    pub fn confidence_history(&self) -> Vec<f64> {
        self.confidence_history.read().clone()
    }

    /// Record a consecutive win/loss streak snapshot for the sparkline trend.
    /// Samples the current streak value and appends it to the rolling history.
    fn record_streak_snapshot(&self) {
        let streak = self
            .current_streak
            .load(std::sync::atomic::Ordering::Relaxed) as f64;
        let mut history = self.streak_history.write();
        history.push(streak);
        if history.len() > MAX_HIT_RATE_HISTORY {
            history.remove(0);
        }
    }

    /// Get the consecutive win/loss streak history for sparkline rendering.
    pub fn streak_history(&self) -> Vec<f64> {
        self.streak_history.read().clone()
    }

    /// Get top performers by win rate (with minimum sample threshold).
    pub fn top_performers(&self, min_samples: u32, limit: usize) -> Vec<PolicyEntry> {
        let mut entries: Vec<_> = self
            .cache
            .read()
            .values()
            .filter(|e| e.sample_size >= min_samples)
            .cloned()
            .collect();
        entries.sort_by(|a, b| {
            b.win_rate()
                .partial_cmp(&a.win_rate())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(limit);
        entries
    }

    /// Compute the 64-bit hash for a feature set.
    fn hash_features(f: &MarketFeatures) -> u64 {
        let mut h = DefaultHasher::new();
        f.hash(&mut h);
        h.finish()
    }

    /// Path to the cache file on disk.
    fn cache_path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".tredo").join("policy_cache.json")
    }
}

// ── Helper Functions ──────────────────────────────────────────────────────

/// Compute a simple confluence score from OHLCV data using existing pivot analysis.
fn compute_confluence_simple(
    symbol: &str,
    current_price: f64,
    bars: &[tredo_core::OhlcvBar],
) -> f64 {
    use tredo_core::{
        calculate_confluence_score, calculate_pivot_points, MarketContext, PivotMethod,
    };

    let high = bars.iter().map(|b| b.high).fold(f64::MIN, f64::max);
    let low = bars.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    let close = bars.last().map(|b| b.close).unwrap_or(current_price);

    let ctx = MarketContext {
        symbol: symbol.to_string(),
        current_price,
        high,
        low,
        previous_close: close,
        timestamp: Utc::now(),
        daily_pnl: 0.0,
        equity: 100_000.0,
        consecutive_losses: 0,
        is_red_folder_day: false,
        trend_direction: None,
    };

    let pivots = calculate_pivot_points(high, low, close, PivotMethod::Classic);
    calculate_confluence_score(&ctx, &pivots)
}

/// Wrapper to reuse existing ATR computation from tredo-autonomous.
mod helpers {
    use tredo_core::OhlcvBar;
    pub fn compute_atr(bars: &[OhlcvBar], period: usize) -> f64 {
        tredo_autonomous::helpers::compute_atr(bars, period)
    }
}
