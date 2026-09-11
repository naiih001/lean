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

    fn sanitize_history(history: &Option<Vec<serde_json::Value>>) -> Option<Vec<serde_json::Value>> {
        history.as_ref().map(|vec| {
            vec.iter().map(|v| {
                let mut val = v.clone();
                // Sanitize multimodal content arrays (image_url -> placeholder)
                if let Some(arr) = val.get("content").and_then(|c| c.as_array()).cloned() {
                    let sanitized: Vec<serde_json::Value> = arr.into_iter().map(|part| {
                        if part.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                            serde_json::json!({"type": "text", "text": "[image omitted — not stored]"})
                        } else { part }
                    }).collect();
                    // If only one text part, collapse to string for SavedMsg compat, but keep array for llm_history
                    // Keep array shape but without images.
                    val["content"] = serde_json::Value::Array(sanitized);
                } else if let Some(s) = val.get("content").and_then(|c| c.as_str()) {
                    if s.contains("<<IMAGE:") || s.contains("<<IMAGE_URL:") {
                        let cleaned = s.replace("<<IMAGE:", "[image omitted").replace("<<IMAGE_URL:", "[image omitted URL");
                        // truncate any long base64 that slipped via string content
                        let truncated = if cleaned.len() > 2000 { format!("{}… [image data stripped]", &cleaned[..2000]) } else { cleaned };
                        val["content"] = serde_json::Value::String(truncated);
                    }
                }
                val
            }).collect()
        })
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        self.updated_at = now_unix();
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.file_path();
        // Do not persist raw image base64 — strip before serializing
        let mut to_save = self.clone();
        to_save.llm_history = Self::sanitize_history(&self.llm_history);
        let json = serde_json::to_string_pretty(&to_save)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    pub fn save_sync(&self) -> anyhow::Result<()> {
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let mut to_save = self.clone();
        to_save.llm_history = Self::sanitize_history(&self.llm_history);
        let json = serde_json::to_string_pretty(&to_save)?;
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
