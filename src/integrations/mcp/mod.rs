pub mod config;
pub mod registry;
pub mod transport;

pub use config::{load_config, ServerConfig};
pub use registry::{
    apply_global_disabled, is_global_disabled, is_server_disabled, set_global_disabled, snapshot,
    toggle_server_disabled, McpServerInfo, McpToolInfo, ServerStatus,
};
pub use transport::{init, reconnect};

use crate::integrations::mcp::registry::{invalidate_tool_cache, registry_lock, tool_cache_lock};
use crate::integrations::mcp::transport::sanitize_schema;
use serde_json::Value;

pub async fn mcp_tool_definitions() -> Vec<Value> {
    if registry::is_global_disabled() {
        return Vec::new();
    }
    {
        let cache = registry::tool_cache_lock().lock().unwrap();
        if let Some(cached) = &*cache {
            return cached.clone();
        }
    }
    let snap = registry::snapshot();
    let mut out = Vec::new();
    for srv in snap
        .iter()
        .filter(|s| matches!(s.status, registry::ServerStatus::Connected))
    {
        if registry::is_server_disabled(&srv.name) {
            continue;
        }
        for tool in &srv.tools {
            let raw_params = if tool.input_schema.is_null()
                || tool.input_schema == Value::Object(Default::default())
            {
                serde_json::json!({"type":"object","properties":{}})
            } else {
                tool.input_schema.clone()
            };
            let mut params = if raw_params.get("type").is_none() {
                let mut m = serde_json::Map::new();
                m.insert("type".into(), Value::String("object".into()));
                m.insert("properties".into(), raw_params);
                Value::Object(m)
            } else {
                raw_params
            };
            params = sanitize_schema(params);
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
    *registry::tool_cache_lock().lock().unwrap() = Some(out.clone());
    out
}

pub async fn call_tool(server: &str, tool: &str, args: Value) -> Result<String, String> {
    if registry::is_server_disabled(server) {
        return Err(format!("MCP server '{}' is disabled (toggled off in /mcp). Re-enable via /mcp to use its tools.", server));
    }
    let peer_opt = {
        let lock = registry::registry_lock().read().unwrap();
        lock.get(server).and_then(|e| e.peer.clone())
    };
    let peer =
        peer_opt.ok_or_else(|| format!("MCP server not found or not connected: {}", server))?;
    let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
    if let Some(obj) = args.as_object() {
        params.arguments = Some(obj.clone());
    } else if args.is_object() {
    } else if !args.is_null() {
        if args != Value::Null {}
    }
    let result = peer
        .call_tool(params)
        .await
        .map_err(|e| format!("MCP call error: {}", e))?;
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
    Ok(crate::tools::truncate_output(
        &out,
        crate::tools::TruncateStrategy::Head,
    ))
}
