use crate::integrations::mcp::config::{load_config, validate_config, ServerConfig};
use crate::integrations::mcp::registry::{
    invalidate_tool_cache, is_global_disabled, is_server_disabled, registry_lock, LiveEntry,
    McpServerInfo, McpToolInfo, ServerStatus,
};
use serde_json::Value;
use std::collections::HashMap;

pub(crate) fn sanitize_pattern(pat: &str) -> String {
    // OpenAI strict validator rejects \0 (null byte) in ECMA regex
    pat.replace("\\0", "")
        .replace("\\x00", "")
        .replace('\0', "")
}

pub(crate) fn sanitize_schema(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            // Strip JSON Schema meta keys that OpenAI's strict validator rejects
            map.remove("$schema");
            map.remove("$id");
            map.remove("$defs");
            map.remove("definitions");
            // Sanitize pattern containing \0
            if let Some(pat) = map
                .get("pattern")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            {
                let cleaned = sanitize_pattern(&pat);
                if cleaned != pat {
                    if cleaned.is_empty() {
                        map.remove("pattern");
                    } else {
                        map.insert("pattern".to_string(), Value::String(cleaned));
                    }
                }
                if let Some(p) = map.get("pattern").and_then(|v| v.as_str()) {
                    if p.contains('\0') {
                        map.remove("pattern");
                    }
                }
            }
            // Strict requires "type":"object" when "properties" present
            if map.contains_key("properties") && !map.contains_key("type") {
                map.insert("type".to_string(), Value::String("object".to_string()));
            }
            // Recursively sanitize properties
            if let Some(props) = map.get("properties").cloned() {
                if let Some(obj) = props.as_object() {
                    let mut sanitized = serde_json::Map::new();
                    for (k, v) in obj {
                        sanitized.insert(k.clone(), sanitize_schema(v.clone()));
                    }
                    map.insert("properties".to_string(), Value::Object(sanitized));
                }
            }
            // Sanitize patternProperties
            if let Some(pp) = map.get("patternProperties").cloned() {
                if let Some(obj) = pp.as_object() {
                    let mut sanitized = serde_json::Map::new();
                    for (k, v) in obj {
                        sanitized.insert(k.clone(), sanitize_schema(v.clone()));
                    }
                    map.insert("patternProperties".to_string(), Value::Object(sanitized));
                }
            }
            // Sanitize single-schema keywords
            for key in [
                "items",
                "not",
                "propertyNames",
                "contains",
                "if",
                "then",
                "else",
            ] {
                if let Some(v) = map.get(key).cloned() {
                    if v.is_object() || v.is_array() {
                        map.insert(key.to_string(), sanitize_schema(v));
                    }
                }
            }
            // additionalProperties may be bool or schema
            if let Some(v) = map.get("additionalProperties").cloned() {
                if v.is_object() || v.is_array() {
                    map.insert("additionalProperties".to_string(), sanitize_schema(v));
                }
            }
            // Sanitize array-schema keywords
            for key in ["anyOf", "allOf", "oneOf", "prefixItems"] {
                if let Some(arr) = map.get(key).cloned() {
                    if let Some(a) = arr.as_array() {
                        let sanitized: Vec<Value> =
                            a.iter().cloned().map(sanitize_schema).collect();
                        map.insert(key.to_string(), Value::Array(sanitized));
                    }
                }
            }
            // Clean any nested schema-like objects under other keys (e.g. Gmail's inlineImages items)
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if [
                    "type",
                    "description",
                    "enum",
                    "pattern",
                    "minLength",
                    "maxLength",
                    "minimum",
                    "maximum",
                    "required",
                    "title",
                    "examples",
                    "default",
                    "format",
                    "properties",
                    "items",
                    "anyOf",
                    "allOf",
                    "oneOf",
                    "prefixItems",
                    "additionalProperties",
                    "patternProperties",
                    "not",
                    "propertyNames",
                    "contains",
                    "if",
                    "then",
                    "else",
                ]
                .contains(&k.as_str())
                {
                    continue;
                }
                if let Some(v) = map.get(&k).cloned() {
                    if v.is_object() {
                        if let Some(obj) = v.as_object() {
                            if obj.contains_key("type")
                                || obj.contains_key("properties")
                                || obj.contains_key("pattern")
                                || obj.contains_key("enum")
                                || obj.contains_key("anyOf")
                            {
                                map.insert(k, sanitize_schema(v));
                            }
                        }
                    } else if v.is_array() {
                        if let Some(arr) = v.as_array() {
                            if arr.iter().any(|e| {
                                e.is_object()
                                    && e.as_object()
                                        .map(|o| {
                                            o.contains_key("type") || o.contains_key("properties")
                                        })
                                        .unwrap_or(false)
                            }) {
                                let sanitized: Vec<Value> =
                                    arr.iter().cloned().map(sanitize_schema).collect();
                                map.insert(k, Value::Array(sanitized));
                            }
                        }
                    }
                }
            }
            Value::Object(map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sanitize_schema).collect()),
        other => other,
    }
}

// ── Tool definitions (cached) ──────────────────────────────────

pub(crate) fn resolve_stdio_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    // If using npx -y @modelcontextprotocol/server-github, try to use global install to avoid 3s npx overhead
    if cmd == "npx"
        && args
            .iter()
            .any(|a| a.contains("@modelcontextprotocol/server-github"))
    {
        // Try npm root -g
        if let Ok(output) = std::process::Command::new("npm")
            .arg("root")
            .arg("-g")
            .output()
        {
            if output.status.success() {
                let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let candidate = std::path::Path::new(&root)
                    .join("@modelcontextprotocol/server-github/dist/index.js");
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

pub(crate) async fn connect_stdio(
    name: String,
    cfg: ServerConfig,
) -> Result<
    (
        rmcp::service::Peer<rmcp::RoleClient>,
        rmcp::service::RunningService<rmcp::RoleClient, ()>,
    ),
    String,
> {
    use rmcp::transport::TokioChildProcess;
    use rmcp::ServiceExt;

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

    let service = ()
        .serve(transport)
        .await
        .map_err(|e| format!("MCP initialize failed for {}: {}", name, e))?;
    let peer = service.peer().clone();
    Ok((peer, service))
}

pub(crate) async fn connect_http(
    name: String,
    cfg: ServerConfig,
) -> Result<
    (
        rmcp::service::Peer<rmcp::RoleClient>,
        rmcp::service::RunningService<rmcp::RoleClient, ()>,
    ),
    String,
> {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::ServiceExt;

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

    let service = ()
        .serve(transport)
        .await
        .map_err(|e| format!("MCP HTTP initialize failed for {}: {}", name, e))?;
    let peer = service.peer().clone();
    Ok((peer, service))
}

pub(crate) async fn connect_one(name: String, cfg: ServerConfig) {
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
                    let tool_infos: Vec<McpToolInfo> = tools
                        .into_iter()
                        .map(|t| McpToolInfo {
                            name: t.name.to_string(),
                            description: t.description.map(|d| d.to_string()),
                            input_schema: {
                                let v = serde_json::to_value(&t.input_schema)
                                    .unwrap_or(Value::Object(Default::default()));
                                if v.is_null() {
                                    serde_json::json!({"type":"object","properties":{}})
                                } else {
                                    v
                                }
                            },
                        })
                        .collect();
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
            lock.insert(
                name.clone(),
                LiveEntry {
                    info: McpServerInfo {
                        name: name.clone(),
                        config: server_cfg,
                        status: ServerStatus::Disabled,
                        error_detail: Some("globally disabled via /mcp".into()),
                        tools: Vec::new(),
                    },
                    peer: None,
                    _service: None,
                },
            );
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
            lock.insert(
                name.clone(),
                LiveEntry {
                    info: McpServerInfo {
                        name: name.clone(),
                        config: server_cfg,
                        status: if disabled {
                            ServerStatus::Disabled
                        } else {
                            ServerStatus::Connecting
                        },
                        error_detail: if disabled {
                            Some("disabled via /mcp toggle".into())
                        } else {
                            None
                        },
                        tools: Vec::new(),
                    },
                    peer: None,
                    _service: None,
                },
            );
        }
    }
    if cfg.is_empty() {
        invalidate_tool_cache();
        return;
    }
    // Filter out disabled for spawning
    let to_spawn: Vec<(String, ServerConfig)> = cfg
        .into_iter()
        .filter(|(n, _)| !is_server_disabled(n))
        .collect();
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
    })
    .await;
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
            ServerStatus::Connected => Ok(format!(
                "{} connected ({} tools)",
                info.name,
                info.tools.len()
            )),
            ServerStatus::Error(ref e) => Err(format!("{} failed: {}", info.name, e)),
            _ => Ok(format!("{} status: {}", info.name, info.status.as_str())),
        }
    } else {
        Err("reconnect failed".into())
    }
}
