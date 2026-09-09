use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Config types ───────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct FileConfig {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, ServerConfig>,
}

// ── Status / Tool types ────────────────────────────────────────

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

// Internal: holds live peer
struct LiveEntry {
    info: McpServerInfo,
    // Keep the RunningService alive; we store it boxed as type-erased via keeping the peer.
    // The peer is cloneable and does the RPCs. The RunningService must stay alive in this struct.
    // We use dynamic dispatch: store Peer + JoinHandle for cleanup
    peer: Option<rmcp::service::Peer<rmcp::RoleClient>>,
    // Keep running service to prevent drop; we store it as boxed future handle
    _service: Option<rmcp::service::RunningService<rmcp::RoleClient, ()>>,
}

static REGISTRY: OnceLock<RwLock<HashMap<String, LiveEntry>>> = OnceLock::new();

fn registry_lock() -> &'static RwLock<HashMap<String, LiveEntry>> {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

// ── Cache for merged tool definitions ───────────────────────────
static TOOL_CACHE: OnceLock<Mutex<Option<Vec<Value>>>> = OnceLock::new();
fn tool_cache_lock() -> &'static Mutex<Option<Vec<Value>>> {
    TOOL_CACHE.get_or_init(|| Mutex::new(None))
}
fn invalidate_tool_cache() {
    if let Some(lock) = TOOL_CACHE.get() {
        *lock.lock().unwrap() = None;
    }
}

// ── Disabled persistence ───────────────────────────────────────
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
            std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
        } else {
            DisabledState::default()
        };
        Mutex::new(state)
    })
}
fn disabled_state_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("mcp_state.json")
}
fn save_disabled_state() {
    if let Some(lock) = DISABLED.get() {
        let state = lock.lock().unwrap().clone();
        let path = disabled_state_path();
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
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
    // update registry
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
pub fn set_server_disabled(name: &str, disabled: bool) {
    {
        let mut st = disabled_lock().lock().unwrap();
        if disabled { st.disabled_servers.insert(name.to_string()); } else { st.disabled_servers.remove(name); }
    }
    save_disabled_state();
    invalidate_tool_cache();
    // update registry immediately
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
}
/// Apply global disable to registry
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
                    // will be reconnected via init
                    entry.info.status = ServerStatus::Connecting;
                    entry.info.error_detail = None;
                }
            }
        }
    }
    invalidate_tool_cache();
}

// ── Path helpers ───────────────────────────────────────────────

fn global_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("mcp.json")
}

fn project_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".lean")
        .join("mcp.json")
}

// ── Env expansion ──────────────────────────────────────────────

fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                // ${VAR}
                if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                    let var: String = chars[i + 2..i + 2 + end].iter().collect();
                    let val = std::env::var(var.trim()).unwrap_or_default();
                    out.push_str(&val);
                    i += 3 + end;
                    continue;
                }
            } else if i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') {
                // $VAR
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let var: String = chars[i + 1..j].iter().collect();
                let val = std::env::var(&var).unwrap_or_default();
                out.push_str(&val);
                i = j;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn expand_server_config(mut cfg: ServerConfig) -> ServerConfig {
    if let Some(ref mut env) = cfg.env {
        for v in env.values_mut() {
            *v = expand_env(v);
        }
    }
    if let Some(ref mut headers) = cfg.headers {
        for v in headers.values_mut() {
            *v = expand_env(v);
        }
    }
    if let Some(ref mut url) = cfg.url {
        *url = expand_env(url);
    }
    if let Some(ref mut cmd) = cfg.command {
        *cmd = expand_env(cmd);
    }
    if let Some(ref mut args) = cfg.args {
        for a in args.iter_mut() {
            *a = expand_env(a);
        }
    }
    cfg
}

// ── Config loading ─────────────────────────────────────────────

fn load_file(path: &PathBuf) -> Option<FileConfig> {
    let txt = std::fs::read_to_string(path).ok()?;
    if txt.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<FileConfig>(&txt).ok()
}

/// Load and merge global + project configs. Project overrides globally on same key.
pub fn load_config() -> HashMap<String, ServerConfig> {
    let mut out: HashMap<String, ServerConfig> = HashMap::new();

    if let Some(fc) = load_file(&global_path()) {
        for (k, v) in fc.mcp_servers {
            out.insert(k, expand_server_config(v));
        }
    }
    if let Some(fc) = load_file(&project_path()) {
        for (k, v) in fc.mcp_servers {
            out.insert(k, expand_server_config(v));
        }
    }
    out
}

fn validate_config(name: &str, cfg: &ServerConfig) -> Option<String> {
    let has_cmd = cfg.command.is_some();
    let has_url = cfg.url.is_some();
    if has_cmd && has_url {
        return Some(format!("{}: cannot have both `command` and `url`", name));
    }
    if !has_cmd && !has_url {
        return Some(format!("{}: must have either `command` or `url`", name));
    }
    if let Some(url) = &cfg.url {
        if url.trim().is_empty() {
            return Some(format!("{}: url is empty", name));
        }
    }
    if let Some(cmd) = &cfg.command {
        if cmd.trim().is_empty() {
            return Some(format!("{}: command is empty", name));
        }
    }
    None
}

// ── Registry snapshot ──────────────────────────────────────────

pub fn snapshot() -> Vec<McpServerInfo> {
    let lock = registry_lock().read().unwrap();
    let mut v: Vec<McpServerInfo> = lock.values().map(|e| {
        let mut info = e.info.clone();
        if is_server_disabled(&info.name) && !matches!(info.status, ServerStatus::Disabled) {
            info.status = ServerStatus::Disabled;
            info.error_detail = Some("disabled via /mcp toggle".into());
        }
        info
    }).collect();
    // Also include disabled servers that are in config but not in registry? registry already has all
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn status_summary() -> String {
    let snap = snapshot();
    if snap.is_empty() {
        return "mcp:0".to_string();
    }
    let connected = snap.iter().filter(|s| matches!(s.status, ServerStatus::Connected)).count();
    let total = snap.len();
    let err = snap.iter().filter(|s| matches!(s.status, ServerStatus::Error(_))).count();
    if err > 0 {
        format!("mcp:{}/{} ({} err)", connected, total, err)
    } else {
        format!("mcp:{}/{}", connected, total)
    }
}

// ── Tool definitions (cached) ──────────────────────────────────

pub async fn mcp_tool_definitions() -> Vec<Value> {
    if is_global_disabled() {
        return Vec::new();
    }
    // fast path: cached
    {
        let cache = tool_cache_lock().lock().unwrap();
        if let Some(cached) = &*cache {
            return cached.clone();
        }
    }
    let snap = snapshot();
    let mut out = Vec::new();
    for srv in snap.iter().filter(|s| matches!(s.status, ServerStatus::Connected)) {
        if is_server_disabled(&srv.name) { continue; }
        for tool in &srv.tools {
            let params = if tool.input_schema.is_null() || tool.input_schema == Value::Object(Default::default()) {
                serde_json::json!({"type":"object","properties":{}})
            } else {
                tool.input_schema.clone()
            };
            let params = if params.get("type").is_none() {
                let mut m = serde_json::Map::new();
                m.insert("type".into(), Value::String("object".into()));
                m.insert("properties".into(), params);
                Value::Object(m)
            } else {
                params
            };
            out.push(serde_json::json!({
                "type": "function",
                "function": {
                    "name": format!("{}__{}", srv.name, tool.name),
                    "description": tool.description.clone().unwrap_or_default(),
                    "parameters": params
                }
            }));
        }
    }
    // cache it
    *tool_cache_lock().lock().unwrap() = Some(out.clone());
    out
}

// ── Call dispatch ──────────────────────────────────────────────

pub async fn call_tool(server: &str, tool: &str, args: Value) -> Result<String, String> {
    if is_server_disabled(server) {
        return Err(format!("MCP server '{}' is disabled (toggled off in /mcp). Re-enable via /mcp to use its tools.", server));
    }
    // Clone peer out of lock to avoid holding lock across await
    let peer_opt = {
        let lock = registry_lock().read().unwrap();
        lock.get(server).and_then(|e| e.peer.clone())
    };
    let peer = peer_opt.ok_or_else(|| format!("MCP server not found or not connected: {}", server))?;

    // Build CallToolRequestParams
    let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
    if let Some(obj) = args.as_object() {
        params.arguments = Some(obj.clone());
    } else if args.is_object() {
        // already handled
    } else if !args.is_null() {
        // wrap non-object as json? For now return error if not object
        // But allow empty
        if args != Value::Null {
            // try to coerce: if args is string "{}", ignore
        }
    }

    let result = peer
        .call_tool(params)
        .await
        .map_err(|e| format!("MCP call error: {}", e))?;

    // result is CallToolResult with content Vec<ContentBlock>
    // Convert to string similar to tools.rs
    let mut out = String::new();
    let is_error = result.is_error.unwrap_or(false);
    if is_error {
        out.push_str("[MCP tool error]\n");
    }
    for block in result.content {
        match block {
            rmcp::model::ContentBlock::Text(t) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t.text);
            }
            rmcp::model::ContentBlock::Image(img) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("<<IMAGE:{}:{}>>", img.mime_type, img.data));
            }
            rmcp::model::ContentBlock::Audio(a) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("[audio {}]", a.mime_type));
            }
            rmcp::model::ContentBlock::Resource(r) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                match r.resource {
                    rmcp::model::ResourceContents::TextResourceContents { uri, text, .. } => {
                        out.push_str(&format!("[resource {}] {}", uri, text));
                    }
                    rmcp::model::ResourceContents::BlobResourceContents { uri, blob, .. } => {
                        out.push_str(&format!("[resource {} blob {}]", uri, blob));
                    }
                    _ => {
                        out.push_str("[resource unknown]");
                    }
                }
            }
            rmcp::model::ContentBlock::ResourceLink(link) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("[resource_link {}]", link.uri));
            }
            _ => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("{:?}", block));
            }
        }
    }
    if out.is_empty() {
        out.push_str("[MCP tool returned no content]");
    }
    // Truncate via same logic as tools.rs (reuse truncate)
    Ok(crate::tools::truncate_output(&out, crate::tools::TruncateStrategy::Head))
}

// ── Connection ─────────────────────────────────────────────────

fn resolve_stdio_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    // If using npx -y @modelcontextprotocol/server-github, try to use global install to avoid 3s npx overhead
    if cmd == "npx" && args.iter().any(|a| a.contains("@modelcontextprotocol/server-github")) {
        // Try npm root -g
        if let Ok(output) = std::process::Command::new("npm").arg("root").arg("-g").output() {
            if output.status.success() {
                let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let candidate = std::path::Path::new(&root).join("@modelcontextprotocol/server-github/dist/index.js");
                if candidate.exists() {
                    // Prefer direct node
                    return ("node".to_string(), vec![candidate.display().to_string()]);
                }
            }
        }
        // Try common global path
        if let Ok(home) = std::env::var("HOME") {
            let p = std::path::Path::new(&home).join(".npm/_npx");
            // fallback to npx
        }
    }
    (cmd.to_string(), args.to_vec())
}

async fn connect_stdio(name: String, cfg: ServerConfig) -> Result<(rmcp::service::Peer<rmcp::RoleClient>, rmcp::service::RunningService<rmcp::RoleClient, ()>), String> {
    use rmcp::ServiceExt;
    use rmcp::transport::TokioChildProcess;

    let raw_cmd = cfg.command.clone().unwrap_or_default();
    let raw_args = cfg.args.clone().unwrap_or_default();
    let env = cfg.env.clone().unwrap_or_default();
    let (cmd, args) = resolve_stdio_command(&raw_cmd, &raw_args);
    let mut command = tokio::process::Command::new(&cmd);
    command.args(&args);
    for (k, v) in env {
        command.env(k, v);
    }
    // Use the builder so we can discard the child's stderr. rmcp's default
    // TokioChildProcess inherits stderr, and MCP servers like
    // @modelcontextprotocol/server-github print "GitHub MCP Server running on
    // stdio" (plus debug logs) to stderr — if inherited, those lines write
    // straight to the terminal and corrupt the TUI.
    let (transport, _stderr) = TokioChildProcess::builder(command)
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn {}: {}", cmd, e))?;

    let service = ().serve(transport).await.map_err(|e| format!("MCP initialize failed for {}: {}", name, e))?;
    let peer = service.peer().clone();
    Ok((peer, service))
}

async fn connect_http(name: String, cfg: ServerConfig) -> Result<(rmcp::service::Peer<rmcp::RoleClient>, rmcp::service::RunningService<rmcp::RoleClient, ()>), String> {
    use rmcp::ServiceExt;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let url = cfg.url.clone().unwrap_or_default();
    let headers = cfg.headers.clone().unwrap_or_default();

    let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());

    // Build custom headers
    let mut custom_headers: HashMap<http::HeaderName, http::HeaderValue> = HashMap::new();
    let mut auth_header: Option<String> = None;
    for (k, v) in headers {
        if k.to_lowercase() == "authorization" && v.to_lowercase().starts_with("bearer ") {
            auth_header = Some(v[7..].trim().to_string());
            continue;
        }
        let hname = http::HeaderName::from_bytes(k.as_bytes())
            .map_err(|_| format!("invalid header name: {}", k))?;
        let hval = http::HeaderValue::from_str(&v)
            .map_err(|_| format!("invalid header value for {}: {}", k, v))?;
        custom_headers.insert(hname, hval);
    }
    if let Some(tok) = auth_header {
        config = config.auth_header(tok);
    }
    if !custom_headers.is_empty() {
        config = config.custom_headers(custom_headers);
    }

    let transport = if config.custom_headers.is_empty() && config.auth_header.is_none() {
        StreamableHttpClientTransport::from_uri(url.clone())
    } else {
        StreamableHttpClientTransport::from_config(config)
    };

    let service = ().serve(transport).await.map_err(|e| format!("MCP HTTP initialize failed for {}: {}", name, e))?;
    let peer = service.peer().clone();
    Ok((peer, service))
}

async fn connect_one(name: String, cfg: ServerConfig) {
    if is_server_disabled(&name) {
        let mut lock = registry_lock().write().unwrap();
        if let Some(entry) = lock.get_mut(&name) {
            entry.info.status = ServerStatus::Disabled;
            entry.info.error_detail = Some("disabled via /mcp toggle".into());
            entry.peer = None;
            entry._service = None;
        }
        return;
    }
    // Validate
    if let Some(err) = validate_config(&name, &cfg) {
        let mut lock = registry_lock().write().unwrap();
        if let Some(entry) = lock.get_mut(&name) {
            entry.info.status = ServerStatus::Error(err.clone());
            entry.info.error_detail = Some(err);
        }
        return;
    }

    // Mark connecting
    {
        let mut lock = registry_lock().write().unwrap();
        if let Some(entry) = lock.get_mut(&name) {
            entry.info.status = ServerStatus::Connecting;
            entry.info.error_detail = None;
        }
    }

    let res = if cfg.command.is_some() {
        connect_stdio(name.clone(), cfg.clone()).await
    } else {
        connect_http(name.clone(), cfg.clone()).await
    };

    match res {
        Ok((peer, service)) => {
            // List tools
            let tools_res = peer.list_all_tools().await;
            match tools_res {
                Ok(tools) => {
                    let tool_infos: Vec<McpToolInfo> = tools.into_iter().map(|t| McpToolInfo {
                        name: t.name.to_string(),
                        description: t.description.map(|d| d.to_string()),
                        input_schema: {
                            let v = serde_json::to_value(&t.input_schema).unwrap_or(Value::Object(Default::default()));
                            if v.is_null() { serde_json::json!({"type":"object","properties":{}}) } else { v }
                        },
                    }).collect();
                    let mut lock = registry_lock().write().unwrap();
                    if let Some(entry) = lock.get_mut(&name) {
                        entry.peer = Some(peer);
                        entry._service = Some(service);
                        entry.info.status = ServerStatus::Connected;
                        entry.info.tools = tool_infos;
                        entry.info.error_detail = None;
                    }
                    drop(lock);
                    invalidate_tool_cache();
                }
                Err(e) => {
                    let err = format!("list_tools failed: {}", e);
                    let mut lock = registry_lock().write().unwrap();
                    if let Some(entry) = lock.get_mut(&name) {
                        // Keep peer/service even if list fails? Still mark error
                        entry.peer = Some(peer);
                        entry._service = Some(service);
                        entry.info.status = ServerStatus::Error(err.clone());
                        entry.info.error_detail = Some(err);
                    }
                    drop(lock);
                    invalidate_tool_cache();
                }
            }
        }
        Err(e) => {
            let mut lock = registry_lock().write().unwrap();
            if let Some(entry) = lock.get_mut(&name) {
                entry.info.status = ServerStatus::Error(e.clone());
                entry.info.error_detail = Some(e);
                entry.peer = None;
                entry._service = None;
            }
            drop(lock);
            invalidate_tool_cache();
        }
    }
}

/// Eager init: spawn connections for all configured servers (respects toggle)
pub async fn init() {
    // If globally disabled, still populate registry but mark Disabled
    if is_global_disabled() {
        let cfg = load_config();
        let mut lock = registry_lock().write().unwrap();
        lock.clear();
        for (name, server_cfg) in cfg {
            lock.insert(name.clone(), LiveEntry {
                info: McpServerInfo {
                    name: name.clone(),
                    config: server_cfg,
                    status: ServerStatus::Disabled,
                    error_detail: Some("globally disabled via /mcp".into()),
                    tools: Vec::new(),
                },
                peer: None,
                _service: None,
            });
        }
        invalidate_tool_cache();
        return;
    }
    let cfg = load_config();
    {
        let mut lock = registry_lock().write().unwrap();
        lock.clear();
        for (name, server_cfg) in cfg.clone() {
            let disabled = is_server_disabled(&name);
            lock.insert(name.clone(), LiveEntry {
                info: McpServerInfo {
                    name: name.clone(),
                    config: server_cfg,
                    status: if disabled { ServerStatus::Disabled } else { ServerStatus::Connecting },
                    error_detail: if disabled { Some("disabled via /mcp toggle".into()) } else { None },
                    tools: Vec::new(),
                },
                peer: None,
                _service: None,
            });
        }
    }
    if cfg.is_empty() {
        invalidate_tool_cache();
        return;
    }
    // Filter out disabled for spawning
    let to_spawn: Vec<(String, ServerConfig)> = cfg.into_iter().filter(|(n, _)| !is_server_disabled(n)).collect();
    if to_spawn.is_empty() {
        invalidate_tool_cache();
        return;
    }
    // Spawn each connection concurrently (only enabled)
    let mut handles = Vec::new();
    for (name, server_cfg) in to_spawn {
        let n = name.clone();
        let c = server_cfg.clone();
        handles.push(tokio::spawn(async move {
            connect_one(n, c).await;
        }));
    }
    // Wait for all with timeout 15s overall (don't block forever)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        for h in handles {
            let _ = h.await;
        }
    }).await;
}

pub async fn reconnect(name: &str) -> Result<String, String> {
    let cfg_opt = {
        let lock = registry_lock().read().unwrap();
        lock.get(name).map(|e| e.info.config.clone())
    };
    let cfg = cfg_opt.ok_or_else(|| format!("MCP server not found: {}", name))?;
    // Clear old
    {
        let mut lock = registry_lock().write().unwrap();
        if let Some(entry) = lock.get_mut(name) {
            entry.peer = None;
            entry._service = None;
            entry.info.status = ServerStatus::Connecting;
            entry.info.error_detail = None;
            entry.info.tools.clear();
        }
    }
    connect_one(name.to_string(), cfg).await;
    let snap = {
        let lock = registry_lock().read().unwrap();
        lock.get(name).map(|e| e.info.clone())
    };
    if let Some(info) = snap {
        match info.status {
            ServerStatus::Connected => Ok(format!("{} connected ({} tools)", info.name, info.tools.len())),
            ServerStatus::Error(ref e) => Err(format!("{} failed: {}", info.name, e)),
            _ => Ok(format!("{} status: {}", info.name, info.status.as_str())),
        }
    } else {
        Err("reconnect failed".into())
    }
}

pub fn is_initialized() -> bool {
    // we consider initialized if registry has been populated (even empty)
    REGISTRY.get().is_some()
}
