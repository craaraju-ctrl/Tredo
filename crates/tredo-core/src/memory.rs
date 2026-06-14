use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

const DECISIONS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("decisions");
const STATE_TABLE: TableDefinition<&str, &str> = TableDefinition::new("state");
const EPISODES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("episodes");

pub struct MemoryStore {
    db: Database,
}

impl MemoryStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, redb::Error> {
        let path = path.as_ref();
        // Robust DB open with aggressive stale-lock recovery for redb 1.5 (DatabaseError from create, redb::Error return type).
        // redb uses OS file locks; previous crashes leave "DatabaseAlreadyOpen".
        let mut last_err: Option<redb::Error> = None;
        for attempt in 0..6 {
            // Always clean possible lock files before (re)try
            let _ = std::fs::remove_file(path.with_extension("lock"));
            let _ = std::fs::remove_file(format!("{}.lock", path.display()));
            if let Some(parent) = path.parent() {
                if let Some(stem) = path.file_stem() {
                    let stem_s = stem.to_string_lossy();
                    let _ = std::fs::remove_file(parent.join(format!("{}.lock", stem_s)));
                    let _ = std::fs::remove_file(parent.join(format!("{}-lock", stem_s)));
                    let _ = std::fs::remove_file(parent.join(format!("{}.redb.lock", stem_s)));
                }
            }
            match Database::create(path) {
                Ok(db) => {
                    if attempt > 0 {
                        eprintln!(
                            "[MemoryStore] ✅ Recovered after {} attempts: {}",
                            attempt + 1,
                            path.display()
                        );
                    }
                    let write_txn = db.begin_write()?;
                    {
                        write_txn.open_table(DECISIONS_TABLE)?;
                        write_txn.open_table(STATE_TABLE)?;
                        write_txn.open_table(EPISODES_TABLE)?;
                    }
                    write_txn.commit()?;
                    return Ok(Self { db });
                }
                Err(e) => {
                    // Convert DatabaseError -> redb::Error for our return / storage
                    let converted: redb::Error = e.into();
                    let s = format!("{:?}", converted);
                    let is_lock = s.contains("AlreadyOpen")
                        || s.to_lowercase().contains("already open")
                        || s.contains("lock");
                    last_err = Some(converted);
                    if attempt < 5 && is_lock {
                        if attempt == 0 {
                            eprintln!("[MemoryStore] {} locked (stale proc/unclean). Retrying with lock removal...", path.display());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(
                            350 + (attempt as u64) * 180,
                        ));
                        continue;
                    }
                    if attempt < 3 {
                        // transient FS / other errs: brief retry
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        continue;
                    }
                    return Err(last_err.unwrap());
                }
            }
        }
        Err(last_err.unwrap_or(redb::Error::DatabaseAlreadyOpen))
    }

    pub fn store_decision(&self, key: &str, value: &str) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(DECISIONS_TABLE)?;
            table.insert(key, value)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_decision(&self, key: &str) -> Result<Option<String>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(DECISIONS_TABLE)?;
        let result = table.get(key)?.map(|value| value.value().to_string());
        Ok(result)
    }

    /// Persist structured state (portfolio, goals, tasks) as JSON by key.
    pub fn store_state(&self, key: &str, value: &str) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(STATE_TABLE)?;
            table.insert(key, value)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Load structured state by key.
    pub fn load_state(&self, key: &str) -> Result<Option<String>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(STATE_TABLE)?;
        let result = table.get(key)?.map(|value| value.value().to_string());
        Ok(result)
    }

    // ── Episode Storage ────────────────────────────────────────────────────

    /// Store a full trading episode as JSON, keyed by episode_id.
    pub fn store_episode(&self, episode_id: &str, json: &str) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(EPISODES_TABLE)?;
            table.insert(episode_id, json)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Load a single episode by ID.
    pub fn load_episode(&self, episode_id: &str) -> Result<Option<String>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(EPISODES_TABLE)?;
        let result = table
            .get(episode_id)?
            .map(|value| value.value().to_string());
        Ok(result)
    }

    /// Load all episode IDs stored since a given Unix timestamp.
    pub fn list_episode_ids_since(&self, since_ts: i64) -> Result<Vec<String>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(EPISODES_TABLE)?;
        let prefix = "ep/";
        let mut ids = Vec::new();
        for result in table.iter()? {
            let (key, _) = result?;
            let key_str = key.value().to_string();
            // Extract timestamp from key: "ep/{symbol}/{unix_ts}"
            if let Some(ts_str) = key_str.rsplit('/').next() {
                if let Ok(ts) = ts_str.parse::<i64>() {
                    if ts >= since_ts && key_str.starts_with(prefix) {
                        ids.push(key_str);
                    }
                }
            }
        }
        // Sort by time ascending
        ids.sort();
        Ok(ids)
    }

    /// Load all episodes since a given timestamp, returning parsed JSON strings.
    pub fn load_episodes_since(&self, since_ts: i64) -> Result<Vec<(String, String)>, redb::Error> {
        let ids = self.list_episode_ids_since(since_ts)?;
        let mut episodes = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(json) = self.load_episode(id)? {
                episodes.push((id.clone(), json));
            }
        }
        Ok(episodes)
    }
}
