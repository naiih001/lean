use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedMsg {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub model: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<SavedMsg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_history: Option<Vec<serde_json::Value>>,
}

fn sessions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("sessions")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_id() -> String {
    // short id + timestamp, no extra deps
    let ts = now_unix();
    let rand: u32 = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        ^ (std::process::id().wrapping_mul(0x9e3779b1)))
        % 0xFFFFFF;
    format!("{:x}-{:06x}", ts, rand)
}

impl Session {
    pub fn new(model: &str) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let now = now_unix();
        Self {
            id: new_id(),
            cwd,
            model: model.to_string(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
            llm_history: None,
        }
    }

    pub fn file_path(&self) -> PathBuf {
        sessions_dir().join(format!("{}.json", self.id))
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        self.updated_at = now_unix();
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.file_path();
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    pub fn save_sync(&self) -> anyhow::Result<()> {
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(self.file_path(), json)?;
        Ok(())
    }

    pub fn load(id: &str) -> anyhow::Result<Self> {
        let path = sessions_dir().join(format!("{}.json", id));
        let s = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn list() -> Vec<Session> {
        let dir = sessions_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(sess) = serde_json::from_str::<Session>(&s) {
                    out.push(sess);
                }
            }
        }
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    pub fn latest() -> Option<Session> {
        Self::list().into_iter().next()
    }

    /// Delete old sessions beyond keep count (default 50)
    pub fn prune(keep: usize) {
        let mut list = Self::list();
        if list.len() <= keep {
            return;
        }
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        for sess in list.into_iter().skip(keep) {
            let _ = std::fs::remove_file(sess.file_path());
        }
    }
}

pub fn session_path_for_id(id: &str) -> PathBuf {
    sessions_dir().join(format!("{}.json", id))
}
