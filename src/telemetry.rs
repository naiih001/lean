use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ActionStat {
    uses: u64,
    last_used_unix: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HotkeyFile {
    version: u8,
    actions: HashMap<String, ActionStat>,
}

static TELEMETRY_INIT: OnceLock<bool> = OnceLock::new();

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn telemetry_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".config"))
        .join("lean")
        .join("hotkey_usage.json")
}

fn load() -> HotkeyFile {
    let path = telemetry_path();
    if path.exists() {
        if let Ok(s) = std::fs::read_to_string(&path) {
            // jcode file may be concatenated JSON (two objects) due to bug; handle by taking first valid
            if let Ok(v) = serde_json::from_str::<HotkeyFile>(&s) {
                return v;
            }
            // try to parse first object before second
            if let Some(end) = s.find("}{") {
                let first = &s[..end + 1];
                if let Ok(v) = serde_json::from_str::<HotkeyFile>(first) {
                    return v;
                }
            }
        }
    }
    HotkeyFile {
        version: 0,
        actions: HashMap::new(),
    }
}

fn save(file: &HotkeyFile) {
    let path = telemetry_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(file) {
        let _ = std::fs::write(&path, json);
    }
}

/// Record a hotkey/interaction usage. Fire-and-forget, never blocks UI.
pub fn record(action: &str) {
    // Cheap: spawn blocking task via std thread to avoid async runtime dependency here
    let action = action.to_string();
    std::thread::spawn(move || {
        let mut file = load();
        let entry = file.actions.entry(action).or_insert(ActionStat {
            uses: 0,
            last_used_unix: now_unix(),
        });
        entry.uses += 1;
        entry.last_used_unix = now_unix();
        save(&file);
    });
}

/// Sync version for when we want to ensure write before exit (rare)
pub fn record_sync(action: &str) {
    let mut file = load();
    let entry = file.actions.entry(action.to_string()).or_insert(ActionStat {
        uses: 0,
        last_used_unix: now_unix(),
    });
    entry.uses += 1;
    entry.last_used_unix = now_unix();
    save(&file);
}
