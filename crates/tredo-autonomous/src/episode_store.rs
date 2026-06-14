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
use std::path::Path;
use std::sync::{Arc, Mutex};

// ── Closed Trade Episode ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ── EpisodeStore ───────────────────────────────────────────────────────────

/// Thread-safe SQLite wrapper for persistent trade history.
///
/// Uses `Arc<Mutex<Connection>>` because rusqlite's Connection is not Send.
/// All writes go through blocking operations — this is fine because SQLite WAL
/// mode makes reads non-blocking and writes are infrequent (only on trade close).
#[derive(Clone)]
pub struct EpisodeStore {
    conn: Arc<Mutex<Connection>>,
}

impl EpisodeStore {
    /// Open or create the SQLite database at `path` and initialise the schema.
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
                            exit_time        TEXT NOT NULL
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
                consecutive_losses_at_entry, entry_time, exit_time
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            params![
                ep.id, ep.symbol, ep.direction, ep.entry_price, ep.exit_price,
                ep.stop_loss, ep.take_profit, ep.position_size, ep.pnl, ep.pnl_pct,
                ep.outcome, ep.exit_reason, ep.regret_score, ep.lesson,
                ep.confluence_score, ep.portfolio_heat, ep.market_regime, ep.session,
                ep.agent_reasoning, ep.consecutive_losses_at_entry,
                ep.entry_time, ep.exit_time,
            ],
        )?;
        Ok(())
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
                    consecutive_losses_at_entry, entry_time, exit_time
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
    })
}
