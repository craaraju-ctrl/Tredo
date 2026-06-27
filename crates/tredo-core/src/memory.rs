use chrono::DateTime;
use serde_json::json;
use std::env;
use std::path::Path;

#[derive(Debug)]
pub struct MemoryStore {
    base_url: String,
    is_online: bool,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug)]
struct ApiRecord {
    id: String,
    content: String,
    content_type: String,
    timestamp: String,
}

impl MemoryStore {
    pub fn new(_path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let base_url =
            env::var("MEMORY_API_URL").unwrap_or_else(|_| "http://localhost:3111".to_string());
        // Try checking health with a short timeout. If offline, it fails instantly or within 10ms.
        let is_online = ureq::get(&format!("{}/health", base_url))
            .timeout(std::time::Duration::from_millis(10))
            .call()
            .is_ok();
        Ok(Self {
            base_url,
            is_online,
        })
    }

    fn store_record(
        &self,
        id: &str,
        content: &str,
        content_type: &str,
    ) -> Result<(), std::io::Error> {
        if !self.is_online {
            return Ok(());
        }
        let url = format!("{}/records", self.base_url);
        let body = json!({
            "id": id,
            "content": content,
            "content_type": content_type,
            "tier": "episodic",
            "importance": 0.5
        });

        match ureq::post(&url).send_json(&body) {
            Ok(resp) => {
                if resp.status() == 200 || resp.status() == 201 {
                    Ok(())
                } else {
                    Err(std::io::Error::other(format!(
                        "Memory service returned status {}",
                        resp.status()
                    )))
                }
            }
            Err(e) => {
                eprintln!("[MemoryStore] ⚠️ Warning: Failed to connect to memory service: {}. Falling back to no-op.", e);
                Ok(())
            }
        }
    }

    fn get_record(&self, id: &str) -> Result<Option<String>, std::io::Error> {
        if !self.is_online {
            return Ok(None);
        }
        let url = format!("{}/records/{}", self.base_url, urlencoding::encode(id));
        match ureq::get(&url).call() {
            Ok(resp) => {
                let record: ApiRecord = resp
                    .into_json()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Some(record.content))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => {
                eprintln!("[MemoryStore] ⚠️ Warning: Failed to connect to memory service: {}. Returning None.", e);
                Ok(None)
            }
        }
    }

    pub fn store_decision(&self, key: &str, value: &str) -> Result<(), std::io::Error> {
        self.store_record(key, value, "decision")
    }

    pub fn get_decision(&self, key: &str) -> Result<Option<String>, std::io::Error> {
        self.get_record(key)
    }

    pub fn store_state(&self, key: &str, value: &str) -> Result<(), std::io::Error> {
        self.store_record(key, value, "state")
    }

    pub fn load_state(&self, key: &str) -> Result<Option<String>, std::io::Error> {
        self.get_record(key)
    }

    pub fn store_episode(&self, episode_id: &str, json: &str) -> Result<(), std::io::Error> {
        self.store_record(episode_id, json, "episode")
    }

    pub fn load_episode(&self, episode_id: &str) -> Result<Option<String>, std::io::Error> {
        self.get_record(episode_id)
    }

    fn get_all_episodes(&self) -> Result<Vec<ApiRecord>, std::io::Error> {
        if !self.is_online {
            return Ok(vec![]);
        }
        let url = format!("{}/records?type=episode&limit=100000", self.base_url);
        match ureq::get(&url).call() {
            Ok(resp) => {
                let records: Vec<ApiRecord> = resp
                    .into_json()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(records)
            }
            Err(e) => {
                eprintln!("[MemoryStore] ⚠️ Warning: Failed to connect to memory service: {}. Returning empty list.", e);
                Ok(vec![])
            }
        }
    }

    pub fn list_episode_ids_since(&self, since_ts: i64) -> Result<Vec<String>, std::io::Error> {
        let records = self.get_all_episodes()?;
        let mut ids = Vec::new();

        for record in records {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&record.timestamp) {
                if dt.timestamp() >= since_ts {
                    ids.push(record.id);
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    pub fn load_episodes_since(
        &self,
        since_ts: i64,
    ) -> Result<Vec<(String, String)>, std::io::Error> {
        let records = self.get_all_episodes()?;
        let mut episodes = Vec::new();

        for record in records {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&record.timestamp) {
                if dt.timestamp() >= since_ts {
                    episodes.push((record.id, record.content));
                }
            }
        }
        episodes.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(episodes)
    }
}
