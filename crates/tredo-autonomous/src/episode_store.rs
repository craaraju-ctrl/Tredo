// episode_store.rs
// EpisodeStore — SQLite-backed storage for trade episodes, COT logs, and regret events.
//
// Two tiers:
//   redb  → hot operational state (portfolio, rules, open episodes) — unchanged
//   SQLite → cold append-only history (closed trades, COT, regret, rule changes)
//
// RAM cost: zero when idle — SQLite pages are only loaded on query.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::weight_tuner::SkillWeightSnapshot;

// ── Closed Trade Episode ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClosedEpisode {
    pub id: String,
    pub symbol: String,
    pub direction: String, // "Long" | "Short"
    pub entry_price: f64,
    pub exit_price: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub position_size: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub outcome: String,     // "WIN" | "LOSS" | "BREAKEVEN"
    pub exit_reason: String, // "stop_loss" | "take_profit" | "manual"
    pub regret_score: f64,   // 0.0 (good) → 1.0 (worst decision)
    pub lesson: String,
    pub confluence_score: f64,
    pub portfolio_heat: f64,
    pub market_regime: String,
    pub session: String,
    pub agent_reasoning: String,
    pub consecutive_losses_at_entry: u32,
    pub entry_time: String, // RFC3339
    pub exit_time: String,  // RFC3339
    /// The rule_version active at the time this episode was generated.
    /// Critical for memory versioning: prevents retrieving episodes from
    /// incompatible rule regimes during vector similarity search.
    pub rule_version: u32,
    /// Whether the trade was profitable (used by EvolvedMetaControl for win rate calculation).
    pub was_correct: bool,
}

// ── Regret Event (high-regret episodes for MetaControl) ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegretEvent {
    pub episode_id: String,
    pub symbol: String,
    pub regret_score: f64,
    pub lesson: String,
    pub rule_violated: String,
    pub recorded_at: String,
}

// ── COT Log Row ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CotLogRow {
    pub chain_id: u64,
    pub agent: String,
    pub action: String,
    pub reason: String,
    pub confidence: f64,
    pub symbol: Option<String>,
    pub ts: String,
}

// ── Rule Change ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleChangeRow {
    pub rule_name: String,
    pub old_value: f64,
    pub new_value: f64,
    pub reason: String,
    pub applied_at: String,
}

/// Snapshot of a rule change for self-evolution reporting (used by SelfEvolutionValidator).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleChangeSnapshot {
    pub rule_name: String,
    pub old_value: f64,
    pub new_value: f64,
    pub reason: String,
    pub applied_at: String,
}

/// Rule snapshot for versioning and rollback in EvolvedMetaControl (exact from user spec).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSnapshot {
    pub version: u32,
    pub config_json: String,
    pub baseline_win_rate: f64,
    pub baseline_avg_regret: f64,
    pub timestamp: u64,
}

// ── Skill Performance (for MetaControl weight tuning) ───────────────────

/// Records which direction a skill predicted and whether it was correct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPerformanceRow {
    pub id: i64,
    pub episode_id: String,
    pub skill_name: String,
    pub direction: String, // "Bullish" | "Bearish" | "Neutral"
    pub weight_used: f64,
    pub confidence: f64,
    pub score: f64,
    /// Whether the skill's direction matched the actual trade direction.
    pub was_correct: bool,
    pub recorded_at: String,
}

// ── EpisodeStore ───────────────────────────────────────────────────────────

/// Thread-safe SQLite wrapper for persistent trade history.
///
/// Uses `Arc<Mutex<Connection>>` because rusqlite's Connection is not Send.
/// All writes go through blocking operations — this is fine because SQLite WAL
/// mode makes reads non-blocking and writes are infrequent (only on trade close).
#[derive(Debug, Clone)]
pub struct EpisodeStore {
    conn: Arc<Mutex<Connection>>,
}

// ── Skill Accuracy Summary ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAccuracySummary {
    pub skill_name: String,
    pub total_votes: usize,
    pub correct_votes: usize,
    pub accuracy: f64,
    pub avg_confidence: f64,
    pub avg_weight: f64,
}

impl EpisodeStore {
    /// Open or create the SQLite database at `path` and initialise the schema.
    /// Stub for test code that expects this method on EpisodeStore.
    pub fn verify_and_initialize_schema(&self) -> Result<(), rusqlite::Error> {
        // The real schema is created in `open`. This is a no-op for in-memory test DBs.
        Ok(())
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, rusqlite::Error> {
        let path = path.as_ref();
        let mut last_err = None;
        for attempt in 0..4 {
            // On retry, clear potential stale WAL/SHM sidecars that can leave DB "locked"
            if attempt > 0 {
                let _ = std::fs::remove_file(path.with_extension("db-shm"));
                let _ = std::fs::remove_file(path.with_extension("db-wal"));
                let _ = std::fs::remove_file("tredo_history.db-shm");
                let _ = std::fs::remove_file("tredo_history.db-wal");
                eprintln!(
                    "[EpisodeStore] Recovery attempt {} for {} (cleaned WAL/SHM)",
                    attempt + 1,
                    path.display()
                );
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            match Connection::open(path) {
                Ok(conn) => {
                    // Enable WAL mode: writes never block reads
                    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

                    // Create tables
                    conn.execute_batch(
                        "
                        CREATE TABLE IF NOT EXISTS closed_trades (
                            id                TEXT PRIMARY KEY,
                            symbol            TEXT NOT NULL,
                            direction         TEXT NOT NULL,
                            entry_price       REAL NOT NULL,
                            exit_price        REAL NOT NULL,
                            stop_loss         REAL NOT NULL,
                            take_profit       REAL NOT NULL,
                            position_size     REAL NOT NULL,
                            pnl               REAL NOT NULL,
                            pnl_pct           REAL NOT NULL,
                            outcome           TEXT NOT NULL,
                            exit_reason       TEXT NOT NULL,
                            regret_score      REAL NOT NULL DEFAULT 0.0,
                            lesson            TEXT NOT NULL DEFAULT '',
                            confluence_score  REAL NOT NULL DEFAULT 0.0,
                            portfolio_heat    REAL NOT NULL DEFAULT 0.0,
                            market_regime     TEXT NOT NULL DEFAULT '',
                            session           TEXT NOT NULL DEFAULT '',
                            agent_reasoning   TEXT NOT NULL DEFAULT '',
                            consecutive_losses_at_entry INTEGER NOT NULL DEFAULT 0,
                            entry_time        TEXT NOT NULL,
                            exit_time        TEXT NOT NULL,
                            rule_version      INTEGER NOT NULL DEFAULT 1,
                            was_correct       INTEGER NOT NULL DEFAULT 0
                        );

                        CREATE INDEX IF NOT EXISTS idx_ct_symbol ON closed_trades(symbol);
                        CREATE INDEX IF NOT EXISTS idx_ct_regime ON closed_trades(market_regime);
                        CREATE INDEX IF NOT EXISTS idx_ct_outcome ON closed_trades(outcome);
                        CREATE INDEX IF NOT EXISTS idx_ct_entry_time ON closed_trades(entry_time);

                        CREATE TABLE IF NOT EXISTS regret_events (
                            id             INTEGER PRIMARY KEY AUTOINCREMENT,
                            episode_id     TEXT NOT NULL,
                            symbol         TEXT NOT NULL,
                            regret_score   REAL NOT NULL,
                            lesson         TEXT NOT NULL DEFAULT '',
                            rule_violated  TEXT NOT NULL DEFAULT '',
                            recorded_at    TEXT NOT NULL
                        );

                        CREATE INDEX IF NOT EXISTS idx_re_symbol ON regret_events(symbol);
                        CREATE INDEX IF NOT EXISTS idx_re_score ON regret_events(regret_score);

                        CREATE TABLE IF NOT EXISTS cot_logs (
                            id         INTEGER PRIMARY KEY AUTOINCREMENT,
                            chain_id   INTEGER NOT NULL,
                            agent      TEXT NOT NULL,
                            action     TEXT NOT NULL,
                            reason     TEXT NOT NULL,
                            confidence REAL NOT NULL,
                            symbol     TEXT,
                            ts         TEXT NOT NULL
                        );

                        CREATE INDEX IF NOT EXISTS idx_cot_chain ON cot_logs(chain_id);
                        CREATE INDEX IF NOT EXISTS idx_cot_ts    ON cot_logs(ts);

                        CREATE TABLE IF NOT EXISTS rule_changes (
                            id         INTEGER PRIMARY KEY AUTOINCREMENT,
                            rule_name  TEXT NOT NULL,
                            old_value  REAL NOT NULL,
                            new_value  REAL NOT NULL,
                            reason     TEXT NOT NULL,
                            applied_at TEXT NOT NULL
                        );

                        CREATE TABLE IF NOT EXISTS skill_performance (
                            id           INTEGER PRIMARY KEY AUTOINCREMENT,
                            episode_id   TEXT NOT NULL,
                            skill_name   TEXT NOT NULL,
                            direction    TEXT NOT NULL,
                            weight_used  REAL NOT NULL,
                            confidence   REAL NOT NULL,
                            score        REAL NOT NULL,
                            was_correct  INTEGER NOT NULL,
                            recorded_at  TEXT NOT NULL
                        );

                        CREATE INDEX IF NOT EXISTS idx_sp_skill ON skill_performance(skill_name);
                        CREATE INDEX IF NOT EXISTS idx_sp_episode ON skill_performance(episode_id);

                        -- Rule snapshots for rollback capability (EvolvedMetaControl regime-conditional evolution and degradation checks)
                        -- Exact columns from user-provided RuleSnapshot
                        CREATE TABLE IF NOT EXISTS rule_snapshots (
                            version           INTEGER PRIMARY KEY,
                            config_json       TEXT NOT NULL,
                            baseline_win_rate REAL NOT NULL,
                            baseline_avg_regret REAL NOT NULL,
                            timestamp         INTEGER NOT NULL
                        );

                        CREATE INDEX IF NOT EXISTS idx_rs_version ON rule_snapshots(version);

                        -- Weight attribution snapshots (from AttributionEngine)
                        CREATE TABLE IF NOT EXISTS skill_weight_snapshots (
                            episode_id      TEXT PRIMARY KEY,
                            initial_weights TEXT NOT NULL,  -- JSON
                            updated_weights TEXT NOT NULL,  -- JSON
                            timestamp       INTEGER NOT NULL
                        );
                    ",
                    )?;

                    if attempt > 0 {
                        eprintln!(
                            "[EpisodeStore] ✅ SQLite recovered after {} attempts (WAL mode)",
                            attempt + 1
                        );
                    } else {
                        println!("[EpisodeStore] ✅ SQLite database ready (WAL mode)");
                    }
                    return Ok(Self {
                        conn: Arc::new(Mutex::new(conn)),
                    });
                }
                Err(e) => {
                    let msg = e.to_string().to_lowercase();
                    last_err = Some(e);
                    if msg.contains("lock")
                        || msg.contains("busy")
                        || msg.contains("database is locked")
                    {
                        continue;
                    }
                    if attempt < 2 {
                        continue;
                    }
                    return Err(last_err.unwrap());
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(5 /* BUSY */),
                Some("unrecoverable lock".into()),
            )
        }))
    }

    // ── Write Operations ───────────────────────────────────────────────────

    /// Insert a completed trade episode.
    pub fn insert_closed_trade(&self, ep: &ClosedEpisode) -> Result<(), rusqlite::Error> {
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        conn.execute(
            "INSERT OR REPLACE INTO closed_trades (
                id, symbol, direction, entry_price, exit_price, stop_loss, take_profit,
                position_size, pnl, pnl_pct, outcome, exit_reason, regret_score, lesson,
                confluence_score, portfolio_heat, market_regime, session, agent_reasoning,
                consecutive_losses_at_entry, entry_time, exit_time, rule_version
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
            params![
                ep.id, ep.symbol, ep.direction, ep.entry_price, ep.exit_price,
                ep.stop_loss, ep.take_profit, ep.position_size, ep.pnl, ep.pnl_pct,
                ep.outcome, ep.exit_reason, ep.regret_score, ep.lesson,
                ep.confluence_score, ep.portfolio_heat, ep.market_regime, ep.session,
                ep.agent_reasoning, ep.consecutive_losses_at_entry,
                ep.entry_time, ep.exit_time, ep.rule_version,
            ],
        )?;
        Ok(())
    }

    pub fn insert_skill_weight_snapshot(&self, snap: &SkillWeightSnapshot) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        conn.execute(
            "INSERT OR REPLACE INTO skill_weight_snapshots (episode_id, initial_weights, updated_weights, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                snap.episode_id,
                serde_json::to_string(&snap.initial_weights).unwrap_or_default(),
                serde_json::to_string(&snap.updated_weights).unwrap_or_default(),
                snap.timestamp as i64
            ],
        )?;
        Ok(())
    }

    /// Load the most recent N market_regime values (for regime stability check in MetaControl).
    pub fn load_recent_regimes(&self, n: usize) -> Result<Vec<String>, rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        let mut stmt = conn.prepare(
            "SELECT market_regime FROM closed_trades ORDER BY entry_time DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![n as i64], |row| row.get(0))?;
        rows.collect()
    }



    /// Load recent closed trades (for performance evaluation and rollback checks).
    /// Filters by rule_version if provided. Includes was_correct for win rate calculation.
    pub fn load_recent_closed_trades(&self, limit: usize, rule_version: Option<u32>) -> Result<Vec<ClosedEpisode>, rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        let sql = if let Some(v) = rule_version {
            format!(
                "SELECT id, symbol, direction, entry_price, exit_price, stop_loss, take_profit, position_size, pnl, pnl_pct, outcome, exit_reason, regret_score, lesson, confluence_score, portfolio_heat, market_regime, session, agent_reasoning, consecutive_losses_at_entry, entry_time, exit_time, rule_version, was_correct FROM closed_trades WHERE rule_version = {} ORDER BY entry_time DESC LIMIT ?1",
                v
            )
        } else {
            "SELECT id, symbol, direction, entry_price, exit_price, stop_loss, take_profit, position_size, pnl, pnl_pct, outcome, exit_reason, regret_score, lesson, confluence_score, portfolio_heat, market_regime, session, agent_reasoning, consecutive_losses_at_entry, entry_time, exit_time, rule_version, was_correct FROM closed_trades ORDER BY entry_time DESC LIMIT ?1".to_string()
        };
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ClosedEpisode {
                id: row.get(0)?,
                symbol: row.get(1)?,
                direction: row.get(2)?,
                entry_price: row.get(3)?,
                exit_price: row.get(4)?,
                stop_loss: row.get(5)?,
                take_profit: row.get(6)?,
                position_size: row.get(7)?,
                pnl: row.get(8)?,
                pnl_pct: row.get(9)?,
                outcome: row.get(10)?,
                exit_reason: row.get(11)?,
                regret_score: row.get(12)?,
                lesson: row.get(13)?,
                confluence_score: row.get(14)?,
                portfolio_heat: row.get(15)?,
                market_regime: row.get(16)?,
                session: row.get(17)?,
                agent_reasoning: row.get(18)?,
                consecutive_losses_at_entry: row.get(19)?,
                entry_time: row.get(20)?,
                exit_time: row.get(21)?,
                rule_version: row.get(22).unwrap_or(0),
                was_correct: row.get::<_, bool>(23).unwrap_or(false),
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get a RuleSnapshot by version for revert logic.
    pub fn get_rule_snapshot(&self, version: u32) -> Result<Option<RuleSnapshot>, rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        let mut stmt = conn.prepare(
            "SELECT version, config_json, baseline_win_rate, baseline_avg_regret, timestamp
             FROM rule_snapshots WHERE version = ?1"
        )?;
        let mut rows = stmt.query_map(params![version], |row| {
            Ok(RuleSnapshot {
                version: row.get(0)?,
                config_json: row.get(1)?,
                baseline_win_rate: row.get(2)?,
                baseline_avg_regret: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;
        rows.next().transpose()
    }

    /// Fetch recent regret scores (for high regret detection in evaluate_and_adapt).
    pub fn fetch_recent_regret_scores(&self, limit: usize) -> Result<Vec<f64>, rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        let mut stmt = conn.prepare(
            "SELECT regret_score FROM closed_trades ORDER BY entry_time DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| row.get(0))?;
        rows.collect()
    }

    /// Insert a new RuleSnapshot for versioning and rollback.
    pub fn insert_rule_snapshot(&self, snapshot: RuleSnapshot) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        conn.execute(
            "INSERT OR REPLACE INTO rule_snapshots (version, config_json, baseline_win_rate, baseline_avg_regret, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                snapshot.version,
                snapshot.config_json,
                snapshot.baseline_win_rate,
                snapshot.baseline_avg_regret,
                snapshot.timestamp as i64
            ],
        )?;
        Ok(())
    }

    /// Record a rule change (supports RULE_REVERT etc.).
    pub fn record_rule_change(&self, rule_name: &str, new_value: &str, reason: &str, timestamp: u64) -> Result<(), rusqlite::Error> {
        let rc = RuleChangeRow {
            rule_name: rule_name.to_string(),
            old_value: 0.0, // caller can provide if needed
            new_value: new_value.parse().unwrap_or(0.0),
            reason: reason.to_string(),
            applied_at: timestamp.to_string(),
        };
        self.insert_rule_change(&rc)
    }

    /// Insert a COT log entry (for meta control reasoning).
    pub fn insert_cot_log(&self, agent: &str, action: &str, reason: &str, timestamp: u64) -> Result<(), rusqlite::Error> {
        let cot = CotLogRow {
            chain_id: 0,
            agent: agent.to_string(),
            action: action.to_string(),
            reason: reason.to_string(),
            confidence: 0.0,
            symbol: None,
            ts: timestamp.to_string(),
        };
        self.insert_cot_log_row(&cot)
    }

    // Helper for cot if needed; assume existing insert_cot_log_row or similar.
    // For compatibility, add a simple insert.
    fn insert_cot_log_row(&self, cot: &CotLogRow) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().expect("lock");
        conn.execute(
            "INSERT INTO cot_logs (chain_id, agent, action, reason, confidence, symbol, ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![cot.chain_id, cot.agent, cot.action, cot.reason, cot.confidence, cot.symbol, cot.ts],
        )?;
        Ok(())
    }

    /// Bridge method used by OutcomeProcessor for the ClosedEpisodeRecord flow.
    /// Maps ClosedEpisodeRecord fields to ClosedEpisode for persistence.
    pub fn close_episode(
        &self,
        record: &crate::outcome_processor::ClosedEpisodeRecord,
        _skill_predictions: &HashMap<String, f64>,
    ) -> Result<(), rusqlite::Error> {
        let ep = ClosedEpisode {
            id: record.episode_id.clone(),
            symbol: record.symbol.clone(),
            direction: record.direction.clone(),
            entry_price: record.entry_price,
            exit_price: record.exit_price,
            stop_loss: 0.0,
            take_profit: 0.0,
            position_size: 0.0,
            pnl: record.raw_pnl,
            pnl_pct: record.pct_pnl,
            outcome: if record.was_correct { "WIN".into() } else { "LOSS".into() },
            exit_reason: "close".into(),
            regret_score: record.regret_score,
            lesson: "".into(),
            confluence_score: 0.0,
            portfolio_heat: 0.0,
            market_regime: "".into(),
            session: "".into(),
            agent_reasoning: "".into(),
            consecutive_losses_at_entry: 0,
            entry_time: record.entry_time.to_string(),
            exit_time: record.exit_time.to_string(),
            rule_version: record.rule_version,
            was_correct: record.was_correct,
        };
        self.insert_closed_trade(&ep)
    }

    /// Insert a high-regret event for MetaControl review.
    pub fn insert_regret_event(&self, ev: &RegretEvent) -> Result<(), rusqlite::Error> {
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        conn.execute(
            "INSERT INTO regret_events (episode_id, symbol, regret_score, lesson, rule_violated, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![ev.episode_id, ev.symbol, ev.regret_score, ev.lesson, ev.rule_violated, ev.recorded_at],
        )?;
        Ok(())
    }

    /// Flush a batch of COT entries to the log table.
    pub fn flush_cot_batch(&self, rows: &[CotLogRow]) -> Result<(), rusqlite::Error> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for r in rows {
            tx.execute(
                "INSERT INTO cot_logs (chain_id, agent, action, reason, confidence, symbol, ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    r.chain_id,
                    r.agent,
                    r.action,
                    r.reason,
                    r.confidence,
                    r.symbol,
                    r.ts
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record a rule change made by MetaControlAgent.
    pub fn insert_rule_change(&self, rc: &RuleChangeRow) -> Result<(), rusqlite::Error> {
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        conn.execute(
            "INSERT INTO rule_changes (rule_name, old_value, new_value, reason, applied_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rc.rule_name,
                rc.old_value,
                rc.new_value,
                rc.reason,
                rc.applied_at
            ],
        )?;
        Ok(())
    }

    // ── Read Operations (Pattern Retriever + MetaControl) ─────────────────

    /// Retrieve the last N similar episodes for a symbol+regime+confluence bucket.
    ///
    /// Confluence buckets: LOW(<0.55), MED(0.55–0.70), HIGH(>0.70)
    pub fn find_similar_episodes(
        &self,
        symbol: &str,
        market_regime: &str,
        confluence_score: f64,
        limit: usize,
    ) -> Result<Vec<ClosedEpisode>, rusqlite::Error> {
        let (conf_min, conf_max) = confluence_bucket(confluence_score);
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        let mut stmt = conn.prepare(
            "SELECT id, symbol, direction, entry_price, exit_price, stop_loss, take_profit,
                    position_size, pnl, pnl_pct, outcome, exit_reason, regret_score, lesson,
                    confluence_score, portfolio_heat, market_regime, session, agent_reasoning,
                    consecutive_losses_at_entry, entry_time, exit_time, rule_version
             FROM closed_trades
             WHERE symbol = ?1
               AND market_regime = ?2
               AND confluence_score BETWEEN ?3 AND ?4
             ORDER BY entry_time DESC
             LIMIT ?5",
        )?;

        let rows = stmt.query_map(
            params![symbol, market_regime, conf_min, conf_max, limit as i64],
            row_to_episode,
        )?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Load high-regret events since a timestamp for MetaControl weekly review.
    pub fn load_regret_events_since(
        &self,
        since: &DateTime<Utc>,
        min_score: f64,
    ) -> Result<Vec<RegretEvent>, rusqlite::Error> {
        let since_str = since.to_rfc3339();
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        let mut stmt = conn.prepare(
            "SELECT episode_id, symbol, regret_score, lesson, rule_violated, recorded_at
             FROM regret_events
             WHERE recorded_at >= ?1 AND regret_score >= ?2
             ORDER BY regret_score DESC",
        )?;

        let rows = stmt.query_map(params![since_str, min_score], |row| {
            Ok(RegretEvent {
                episode_id: row.get(0)?,
                symbol: row.get(1)?,
                regret_score: row.get(2)?,
                lesson: row.get(3)?,
                rule_violated: row.get(4)?,
                recorded_at: row.get(5)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Count regret events today — used to auto-trigger MetaControl.
    pub fn count_regret_events_today(&self) -> usize {
        let today_str = Utc::now().format("%Y-%m-%d").to_string();
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        conn.query_row(
            "SELECT COUNT(*) FROM regret_events WHERE recorded_at LIKE ?1",
            params![format!("{}%", today_str)],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Get the most recently closed trade for a symbol (used by SelfEvolutionValidator).
    pub fn get_most_recent_closed(
        &self,
        symbol: &str,
    ) -> Result<Option<ClosedEpisode>, rusqlite::Error> {
        let conn = self.conn.lock().expect("SQLite connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, symbol, direction, entry_price, exit_price, stop_loss, take_profit,
                    position_size, pnl, pnl_pct, outcome, exit_reason, regret_score, lesson,
                    confluence_score, portfolio_heat, market_regime, session, agent_reasoning,
                    consecutive_losses_at_entry, entry_time, exit_time
             FROM closed_trades
             WHERE symbol = ?1
             ORDER BY exit_time DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![symbol], row_to_episode)?;
        match rows.next() {
            Some(Ok(ep)) => Ok(Some(ep)),
            _ => Ok(None),
        }
    }

    /// Get recent rule changes, limited to `limit` entries (used by SelfEvolutionValidator).
    pub fn get_recent_rule_changes(
        &self,
        limit: usize,
    ) -> Result<Vec<RuleChangeSnapshot>, rusqlite::Error> {
        let conn = self.conn.lock().expect("SQLite connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT rule_name, old_value, new_value, reason, applied_at
             FROM rule_changes
             ORDER BY applied_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(RuleChangeSnapshot {
                rule_name: row.get(0)?,
                old_value: row.get(1)?,
                new_value: row.get(2)?,
                reason: row.get(3)?,
                applied_at: row.get(4)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get all rule changes (used for full report generation).
    pub fn get_all_rule_changes(&self) -> Result<Vec<RuleChangeSnapshot>, rusqlite::Error> {
        let conn = self.conn.lock().expect("SQLite connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT rule_name, old_value, new_value, reason, applied_at
             FROM rule_changes
             ORDER BY applied_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RuleChangeSnapshot {
                rule_name: row.get(0)?,
                old_value: row.get(1)?,
                new_value: row.get(2)?,
                reason: row.get(3)?,
                applied_at: row.get(4)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ── Skill Performance Operations ───────────────────────────────────

    /// Record a single skill vote and whether it was correct vs the trade outcome.
    pub fn insert_skill_performance(
        &self,
        sp: &SkillPerformanceRow,
    ) -> Result<(), rusqlite::Error> {
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        conn.execute(
            "INSERT INTO skill_performance (episode_id, skill_name, direction, weight_used, confidence, score, was_correct, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                sp.episode_id,
                sp.skill_name,
                sp.direction,
                sp.weight_used,
                sp.confidence,
                sp.score,
                sp.was_correct as i64,
                sp.recorded_at,
            ],
        )?;
        Ok(())
    }

    /// Load skill performance records since a timestamp, for MetaControl analysis.
    pub fn load_skill_performance_since(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<SkillPerformanceRow>, rusqlite::Error> {
        let since_str = since.to_rfc3339();
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        let mut stmt = conn.prepare(
            "SELECT id, episode_id, skill_name, direction, weight_used, confidence, score, was_correct, recorded_at
             FROM skill_performance
             WHERE recorded_at >= ?1
             ORDER BY recorded_at DESC",
        )?;
        let rows = stmt.query_map(params![since_str], |row| {
            Ok(SkillPerformanceRow {
                id: row.get(0)?,
                episode_id: row.get(1)?,
                skill_name: row.get(2)?,
                direction: row.get(3)?,
                weight_used: row.get(4)?,
                confidence: row.get(5)?,
                score: row.get(6)?,
                was_correct: row.get::<_, i64>(7)? != 0,
                recorded_at: row.get(8)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get per-skill accuracy statistics since a timestamp.
    pub fn skill_accuracy_stats(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<SkillAccuracySummary>, rusqlite::Error> {
        let since_str = since.to_rfc3339();
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        let mut stmt = conn.prepare(
            "SELECT skill_name,
                    COUNT(*) as total,
                    SUM(was_correct) as correct_count,
                    AVG(confidence) as avg_confidence,
                    AVG(weight_used) as avg_weight
             FROM skill_performance
             WHERE recorded_at >= ?1
             GROUP BY skill_name
             ORDER BY correct_count * 1.0 / COUNT(*) DESC",
        )?;
        let rows = stmt.query_map(params![since_str], |row| {
            let total: i64 = row.get(1)?;
            let correct: i64 = row.get(2)?;
            Ok(SkillAccuracySummary {
                skill_name: row.get(0)?,
                total_votes: total as usize,
                correct_votes: correct as usize,
                accuracy: if total > 0 {
                    correct as f64 / total as f64
                } else {
                    0.0
                },
                avg_confidence: row.get(3)?,
                avg_weight: row.get(4)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Summary statistics for the frontend or dashboards.
    pub fn session_stats(&self) -> SessionStats {
        let conn = self
            .conn
            .lock()
            .expect("SQLite connection lock poisoned - this indicates a previous panic in DB code");
        let today = Utc::now().format("%Y-%m-%d").to_string();

        let total: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM closed_trades WHERE entry_time LIKE ?1",
                params![format!("{}%", today)],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let wins: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM closed_trades WHERE outcome='WIN' AND entry_time LIKE ?1",
                params![format!("{}%", today)],
                |r| r.get(0),
            )
            .unwrap_or(0);

        let total_pnl: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(pnl), 0.0) FROM closed_trades WHERE entry_time LIKE ?1",
                params![format!("{}%", today)],
                |r| r.get(0),
            )
            .unwrap_or(0.0);

        let avg_regret: f64 = conn.query_row(
            "SELECT COALESCE(AVG(regret_score), 0.0) FROM closed_trades WHERE entry_time LIKE ?1",
            params![format!("{}%", today)],
            |r| r.get(0),
        ).unwrap_or(0.0);

        SessionStats {
            trades_today: total as u32,
            wins_today: wins as u32,
            losses_today: (total - wins) as u32,
            win_rate: if total > 0 {
                wins as f64 / total as f64
            } else {
                0.0
            },
            total_pnl,
            avg_regret,
        }
    }
}

// ── Session Stats (returned by session_stats()) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub trades_today: u32,
    pub wins_today: u32,
    pub losses_today: u32,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub avg_regret: f64,
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Map a confluence score to a ±0.08 range bucket for similarity queries.
fn confluence_bucket(score: f64) -> (f64, f64) {
    let half = 0.08_f64;
    ((score - half).max(0.0), (score + half).min(1.0))
}

fn row_to_episode(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClosedEpisode> {
    Ok(ClosedEpisode {
        id: row.get(0)?,
        symbol: row.get(1)?,
        direction: row.get(2)?,
        entry_price: row.get(3)?,
        exit_price: row.get(4)?,
        stop_loss: row.get(5)?,
        take_profit: row.get(6)?,
        position_size: row.get(7)?,
        pnl: row.get(8)?,
        pnl_pct: row.get(9)?,
        outcome: row.get(10)?,
        exit_reason: row.get(11)?,
        regret_score: row.get(12)?,
        lesson: row.get(13)?,
        confluence_score: row.get(14)?,
        portfolio_heat: row.get(15)?,
        market_regime: row.get(16)?,
        session: row.get(17)?,
        agent_reasoning: row.get(18)?,
        consecutive_losses_at_entry: row.get(19)?,
        entry_time: row.get(20)?,
        exit_time: row.get(21)?,
        rule_version: row.get(22).unwrap_or(1),
        was_correct: false, // populated by query-specific logic; default false
    })
}
