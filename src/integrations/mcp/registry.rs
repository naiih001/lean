use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::integrations::mcp::config::ServerConfig;

#[derive(Debug, Clone)]
pub enum ServerStatus {
    Connected,
    Connecting,
    Error(String),
    Disabled,
}

impl ServerStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Connected => "connected",
            Self::Connecting => "connecting",
            Self::Error(_) => "error",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub config: ServerConfig,
    pub status: ServerStatus,
    pub error_detail: Option<String>,
    pub tools: Vec<McpToolInfo>,
}

pub(crate) struct LiveEntry {
    pub(crate) info: McpServerInfo,
    pub(crate) peer: Option<rmcp::service::Peer<rmcp::RoleClient>>,
    pub(crate) _service: Option<rmcp::service::RunningService<rmcp::RoleClient, ()>>,
}

pub(crate) static REGISTRY: OnceLock<RwLock<HashMap<String, LiveEntry>>> = OnceLock::new();

pub(crate) fn registry_lock() -> &'static RwLock<HashMap<String, LiveEntry>> {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

static TOOL_CACHE: OnceLock<Mutex<Option<Vec<Value>>>> = OnceLock::new();
pub(crate) fn tool_cache_lock() -> &'static Mutex<Option<Vec<Value>>> {
    TOOL_CACHE.get_or_init(|| Mutex::new(None))
}
pub(crate) fn invalidate_tool_cache() {
    if let Some(lock) = TOOL_CACHE.get() {
        *lock.lock().unwrap() = None;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DisabledState {
    #[serde(default)]
    global_disabled: bool,
    #[serde(default)]
    disabled_servers: HashSet<String>,
}
static DISABLED: OnceLock<Mutex<DisabledState>> = OnceLock::new();
fn disabled_lock() -> &'static Mutex<DisabledState> {
    DISABLED.get_or_init(|| {
        let path = disabled_state_path();
        let state = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            DisabledState::default()
        };
        Mutex::new(state)
    })
}
fn disabled_state_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("mcp_state.json")
}
fn save_disabled_state() {
    if let Some(lock) = DISABLED.get() {
        let state = lock.lock().unwrap().clone();
        let path = disabled_state_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(path, json);
        }
    }
}
pub fn is_global_disabled() -> bool {
    disabled_lock().lock().unwrap().global_disabled
}
pub fn is_server_disabled(name: &str) -> bool {
    let st = disabled_lock().lock().unwrap();
    st.global_disabled || st.disabled_servers.contains(name)
}
pub fn set_global_disabled(disabled: bool) {
    {
        let mut st = disabled_lock().lock().unwrap();
        st.global_disabled = disabled;
    }
    save_disabled_state();
    apply_global_disabled(disabled);
}
pub fn toggle_server_disabled(name: &str) -> bool {
    let disabled = {
        let mut st = disabled_lock().lock().unwrap();
        if st.disabled_servers.contains(name) {
            st.disabled_servers.remove(name);
            false
        } else {
            st.disabled_servers.insert(name.to_string());
            true
        }
    };
    save_disabled_state();
    invalidate_tool_cache();
    if let Some(lock) = REGISTRY.get() {
        if let Ok(mut map) = lock.write() {
            if let Some(entry) = map.get_mut(name) {
                if disabled {
                    entry.info.status = ServerStatus::Disabled;
                    entry.info.error_detail = Some("disabled via /mcp toggle".into());
                    entry.peer = None;
                    entry._service = None;
                    entry.info.tools.clear();
                } else {
                    entry.info.status = ServerStatus::Connecting;
                    entry.info.error_detail = None;
                }
            }
        }
    }
    disabled
}
pub fn apply_global_disabled(disabled: bool) {
    if let Some(lock) = REGISTRY.get() {
        if let Ok(mut map) = lock.write() {
            for (_, entry) in map.iter_mut() {
                if disabled {
                    entry.info.status = ServerStatus::Disabled;
                    entry.info.error_detail = Some("globally disabled via /mcp".into());
                    entry.peer = None;
                    entry._service = None;
                    entry.info.tools.clear();
                } else {
                    entry.info.status = ServerStatus::Connecting;
                    entry.info.error_detail = None;
                }
            }
        }
    }
    invalidate_tool_cache();
}

pub fn snapshot() -> Vec<McpServerInfo> {
    let lock = registry_lock().read().unwrap();
    let mut v: Vec<McpServerInfo> = lock
        .values()
        .map(|e| {
            let mut info = e.info.clone();
            if is_server_disabled(&info.name) && !matches!(info.status, ServerStatus::Disabled) {
                info.status = ServerStatus::Disabled;
                info.error_detail = Some("disabled via /mcp toggle".into());
            }
            info
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}
