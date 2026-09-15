use crate::services::agents::Agent;
use anyhow::Result;
use dirs::home_dir;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub(crate) const CACHE_TTL: Duration = Duration::from_secs(60);

static CACHE: OnceLock<std::sync::Mutex<Option<(Vec<Agent>, Instant)>>> = OnceLock::new();

pub(crate) fn cache_lock() -> &'static std::sync::Mutex<Option<(Vec<Agent>, Instant)>> {
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

pub(crate) fn parse_frontmatter_agents(
    raw: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<Vec<String>>,
    Option<String>,
    Option<String>,
    Option<Vec<String>>,
    Option<bool>,
    String,
) {
    if !raw.starts_with("---") {
        return (None, None, None, None, None, None, None, raw.to_string());
    }
    let end = raw[3..].find("\n---").map(|i| i + 3);
    let end = match end {
        Some(i) => i,
        None => return (None, None, None, None, None, None, None, raw.to_string()),
    };
    let fm = &raw[3..end].trim();
    let body = raw[end + 4..].trim_start().to_string();
    let mut name = None;
    let mut description = None;
    let mut tools = None;
    let mut model = None;
    let mut thinking = None;
    let mut subagent_agents = None;
    let mut auto_exit = None;
    for line in fm.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val_raw = val.trim();
            let val = val_raw
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| {
                    val_raw
                        .strip_prefix('\'')
                        .and_then(|v| v.strip_suffix('\''))
                })
                .unwrap_or(val_raw)
                .trim();
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                "tools" => {
                    let list: Vec<String> = val
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !list.is_empty() {
                        tools = Some(list);
                    }
                }
                "model" => model = Some(val.to_string()),
                "thinking" => thinking = Some(val.to_string()),
                "subagent_agents" => {
                    let list: Vec<String> = val
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !list.is_empty() {
                        subagent_agents = Some(list);
                    } else {
                        subagent_agents = Some(vec![]);
                    }
                }
                "auto-exit" | "auto_exit" => auto_exit = Some(val == "true"),
                _ => {}
            }
        }
    }
    (
        name,
        description,
        tools,
        model,
        thinking,
        subagent_agents,
        auto_exit,
        body,
    )
}

pub(crate) async fn scan_dir(base: &Path) -> Vec<Agent> {
    let mut agents = Vec::new();
    let read_dir = match tokio::fs::read_dir(base).await {
        Ok(rd) => rd,
        Err(_) => return agents,
    };
    let mut entries = read_dir;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let file_type = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => {
                if let Ok(meta) = std::fs::metadata(entry.path()) {
                    meta.file_type()
                } else {
                    continue;
                }
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let agent_path = entry.path().join("AGENTS.md");
        if !agent_path.exists() {
            continue;
        }
        let raw = match tokio::fs::read_to_string(&agent_path).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let (name_opt, desc_opt, tools, model, _thinking, _subagent_agents, _auto_exit, body) =
            parse_frontmatter_agents(&raw);
        let agent_name =
            name_opt.unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
        let desc = desc_opt.unwrap_or_else(|| {
            body.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .chars()
                .take(140)
                .collect()
        });
        agents.push(Agent {
            name: agent_name,
            description: desc,
            path: agent_path,
            content: raw,
            body,
            tools,
            model,
        });
    }
    agents
}

pub async fn discover_agents() -> Vec<Agent> {
    {
        let lock = cache_lock().lock().unwrap();
        if let Some((cached, time)) = lock.as_ref() {
            if time.elapsed() < CACHE_TTL {
                return cached.clone();
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let bases = vec![
        cwd.join("agents"),
        cwd.join(".lean").join("agents"),
        home.join(".lean").join("agents"),
    ];
    let mut all: HashMap<String, Agent> = HashMap::new();
    for base in &bases {
        let found = scan_dir(base).await;
        let is_local = base.starts_with(&cwd);
        for agent in found {
            if !all.contains_key(&agent.name) || is_local {
                all.insert(agent.name.clone(), agent);
            }
        }
    }
    let mut result: Vec<Agent> = all.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    {
        let mut lock = cache_lock().lock().unwrap();
        *lock = Some((result.clone(), Instant::now()));
    }
    result
}

pub async fn get_agent_catalog() -> String {
    let agents = discover_agents().await;
    if agents.is_empty() {
        return "No agents installed. Add to agents/<name>/AGENTS.md or ~/.lean/agents/<name>/AGENTS.md".to_string();
    }
    agents
        .iter()
        .map(|a| {
            let tools_str = a
                .tools
                .as_ref()
                .map(|t| t.join(", "))
                .unwrap_or_else(|| "all".to_string());
            format!(
                "- {}: {} (tools: {}) (path: {})",
                a.name,
                a.description,
                tools_str,
                a.path.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn load_agent(name: &str) -> Result<Agent> {
    let agents = discover_agents().await;
    if let Some(a) = agents.iter().find(|a| a.name == name) {
        return Ok(a.clone());
    }
    let avail: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    anyhow::bail!("Agent not found: {}. Available: {}", name, avail.join(", "))
}
