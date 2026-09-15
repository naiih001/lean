use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

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
pub(crate) struct FileConfig {
    #[serde(rename = "mcpServers", default)]
    pub(crate) mcp_servers: HashMap<String, ServerConfig>,
}

pub(crate) fn global_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("mcp.json")
}

pub(crate) fn project_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".lean")
        .join("mcp.json")
}

pub(crate) fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                    let var: String = chars[i + 2..i + 2 + end].iter().collect();
                    let val = std::env::var(var.trim()).unwrap_or_default();
                    out.push_str(&val);
                    i += 3 + end;
                    continue;
                }
            } else if i + 1 < chars.len()
                && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_')
            {
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

pub(crate) fn expand_server_config(mut cfg: ServerConfig) -> ServerConfig {
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

pub(crate) fn load_file(path: &PathBuf) -> Option<FileConfig> {
    let txt = std::fs::read_to_string(path).ok()?;
    if txt.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<FileConfig>(&txt).ok()
}

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

pub(crate) fn validate_config(name: &str, cfg: &ServerConfig) -> Option<String> {
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
