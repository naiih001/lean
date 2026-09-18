// Mode registry — opencode-like configurable modes.
//
// Modes live in `~/.lean/modes.json` (global) with per-project overrides in
// `.lean/modes.json`. Each mode customizes the model, temperature, tool
// allowlist, and extra prompt text. Missing files fall back to builtins.
//
// Example:
// {
//   "version": 1,
//   "modes": {
//     "plan": { "temperature": 0.1, "tools": { "write": "plans-only" } },
//     "review": {
//       "description": "Read-only code review",
//       "behavior": "ask",
//       "model": "gpt-4o",
//       "tools": { "write": false, "edit": false, "bash": "readonly", "mcp": "readonly" }
//     }
//   }
// }

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Special key in a mode's `tools` map controlling non-read MCP tools
/// (`server__tool`). MCP reads stay governed by the mode behavior.
pub const MCP_TOOL_KEY: &str = "mcp";

/// Native tool names lean can gate per mode.
const KNOWN_TOOLS: &[&str] = &[
    "read",
    "write",
    "edit",
    "bash",
    "grep",
    "find",
    "ls",
    "web_search",
    "web_fetch",
    "read_skill",
    "read_agent",
    "subagent",
    "subagents_list",
    "ask_user",
    MCP_TOOL_KEY,
];

/// Gate behavior a mode inherits. Custom modes map onto one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateBehavior {
    Norm,
    Plan,
    Ask,
}

/// Per-tool access within a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolAccess {
    Allow,
    Deny,
    /// Only `.lean/plans/` writes (write/edit in plan mode).
    PlansOnly,
    /// Read-only invocations only (bash / MCP in plan & ask modes).
    /// Accepts both "read-only" and "readonly" in config files.
    #[serde(alias = "readonly")]
    ReadOnly,
}

impl ToolAccess {
    fn from_raw(raw: &RawAccess) -> Self {
        match raw {
            RawAccess::Bool(true) => ToolAccess::Allow,
            RawAccess::Bool(false) => ToolAccess::Deny,
            RawAccess::Named(a) => *a,
        }
    }
}

/// Raw per-tool access as written in modes.json: `true`/`false` or a named
/// level ("allow", "deny", "plans-only", "read-only"/"readonly").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawAccess {
    Bool(bool),
    Named(ToolAccess),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModeEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Gate behavior; defaults by mode name (plan→Plan, ask→Ask, else Norm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<GateBehavior>,
    /// Model alias override (resolved via models.json like the session model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Extra prompt text: absolute path or relative to CWD, appended to the base prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<HashMap<String, RawAccess>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModesConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub modes: HashMap<String, ModeEntry>,
}

fn default_version() -> u32 {
    1
}

fn builtin_entries() -> HashMap<String, ModeEntry> {
    let mut m = HashMap::new();
    m.insert("norm".to_string(), ModeEntry::default());
    m.insert(
        "plan".to_string(),
        ModeEntry {
            description: Some("Restricted planning mode".to_string()),
            behavior: Some(GateBehavior::Plan),
            tools: Some(HashMap::from([
                ("write".to_string(), RawAccess::Named(ToolAccess::PlansOnly)),
                ("edit".to_string(), RawAccess::Named(ToolAccess::PlansOnly)),
                ("bash".to_string(), RawAccess::Named(ToolAccess::ReadOnly)),
                (
                    MCP_TOOL_KEY.to_string(),
                    RawAccess::Named(ToolAccess::ReadOnly),
                ),
            ])),
            ..Default::default()
        },
    );
    m.insert(
        "ask".to_string(),
        ModeEntry {
            description: Some("Read-only consultative mode".to_string()),
            behavior: Some(GateBehavior::Ask),
            tools: Some(HashMap::from([
                ("write".to_string(), RawAccess::Bool(false)),
                ("edit".to_string(), RawAccess::Bool(false)),
                ("bash".to_string(), RawAccess::Named(ToolAccess::ReadOnly)),
                (
                    MCP_TOOL_KEY.to_string(),
                    RawAccess::Named(ToolAccess::ReadOnly),
                ),
            ])),
            ..Default::default()
        },
    );
    m
}

fn template_config() -> ModesConfig {
    ModesConfig {
        version: 1,
        modes: builtin_entries(),
    }
}

/// Builtin norm resolution without touching disk (startup default).
pub fn builtin_norm() -> ResolvedMode {
    ResolvedMode {
        name: "norm".to_string(),
        description: "Default build mode".to_string(),
        behavior: GateBehavior::Norm,
        model_alias: None,
        temperature: None,
        prompt_extra: None,
        tools: HashMap::new(),
    }
}

pub fn global_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("modes.json")
}

pub fn project_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".lean")
        .join("modes.json")
}

pub fn path_display() -> String {
    global_path().display().to_string()
}

fn load_file(path: &std::path::Path) -> Result<ModesConfig> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let cfg: ModesConfig = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parse {} as JSON (check syntax with `jq empty {}`)",
            path.display(),
            path.display()
        )
    })?;
    Ok(cfg)
}

/// Load merged config: builtins ← global ← project. Auto-creates the global
/// template when neither file exists (mirrors models.json).
pub fn load() -> Result<ModesConfig> {
    let global = global_path();
    let project = project_path();
    if !global.exists() && !project.exists() {
        if let Some(parent) = global.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let cfg = template_config();
        let json = serde_json::to_string_pretty(&cfg).context("serialize template")?;
        std::fs::write(&global, json)
            .with_context(|| format!("write template {}", global.display()))?;
        return Ok(cfg);
    }
    let mut merged = template_config();
    for path in [global, project] {
        if !path.exists() {
            continue;
        }
        let cfg = load_file(&path)?;
        if cfg.version != 1 {
            anyhow::bail!(
                "{}: unsupported version {} (expected 1) — delete it to regenerate",
                path.display(),
                cfg.version
            );
        }
        for (name, entry) in cfg.modes {
            merged
                .modes
                .entry(name.to_lowercase())
                .and_modify(|e| merge_entry(e, &entry))
                .or_insert(entry);
        }
    }
    Ok(merged)
}

fn merge_entry(base: &mut ModeEntry, over: &ModeEntry) {
    if over.description.is_some() {
        base.description = over.description.clone();
    }
    if over.behavior.is_some() {
        base.behavior = over.behavior;
    }
    if over.model.is_some() {
        base.model = over.model.clone();
    }
    if over.temperature.is_some() {
        base.temperature = over.temperature;
    }
    if over.prompt_file.is_some() {
        base.prompt_file = over.prompt_file.clone();
    }
    if let Some(tools) = &over.tools {
        let map = base.tools.get_or_insert_with(HashMap::new);
        for (k, v) in tools {
            map.insert(k.clone(), *v);
        }
    }
}

/// Fully resolved mode: behavior + tool access + prompt extra.
#[derive(Debug, Clone)]
pub struct ResolvedMode {
    pub name: String,
    pub description: String,
    pub behavior: GateBehavior,
    pub model_alias: Option<String>,
    pub temperature: Option<f32>,
    pub prompt_extra: Option<String>,
    pub tools: HashMap<String, ToolAccess>,
}

fn default_behavior_for(name: &str) -> GateBehavior {
    match name {
        "plan" => GateBehavior::Plan,
        "ask" => GateBehavior::Ask,
        _ => GateBehavior::Norm,
    }
}

fn warn_once(msg: String) {
    static WARNED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    let set = WARNED.get_or_init(|| Mutex::new(std::collections::HashSet::new()));
    let mut guard = set.lock().unwrap_or_else(|e| e.into_inner());
    if guard.insert(msg.clone()) {
        eprintln!("[modes] {}", msg);
    }
}

fn load_prompt_extra(path_str: &str) -> Option<String> {
    let path = PathBuf::from(path_str);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            const MAX: usize = 2000;
            if s.chars().count() > MAX {
                Some(format!(
                    "{}… [truncated]",
                    s.chars().take(MAX).collect::<String>()
                ))
            } else {
                Some(s)
            }
        }
        Err(e) => {
            warn_once(format!("prompt_file {} unreadable: {}", path.display(), e));
            None
        }
    }
}

/// Resolve a mode by name (case-insensitive). Unknown tool keys are ignored
/// with a one-time warning; unknown mode names are an error.
pub fn resolve(name: &str) -> Result<ResolvedMode> {
    let cfg = load()?;
    let key = name.trim().to_lowercase();
    let entry = cfg.modes.get(&key).ok_or_else(|| {
        let mut names: Vec<_> = cfg.modes.keys().cloned().collect();
        names.sort();
        anyhow::anyhow!(
            "unknown mode '{}' (available: {}) — edit {}",
            name,
            names.join(", "),
            path_display()
        )
    })?;
    let mut tools = HashMap::new();
    if let Some(map) = &entry.tools {
        for (k, v) in map {
            let kl = k.to_lowercase();
            if !KNOWN_TOOLS.contains(&kl.as_str()) {
                warn_once(format!(
                    "mode '{}': unknown tool '{}' ignored (known: {})",
                    key,
                    k,
                    KNOWN_TOOLS.join(", ")
                ));
                continue;
            }
            tools.insert(kl, ToolAccess::from_raw(v));
        }
    }
    if let Some(t) = entry.temperature {
        if !(0.0..=2.0).contains(&t) {
            anyhow::bail!("mode '{}': temperature {} out of range 0.0–2.0", key, t);
        }
    }
    Ok(ResolvedMode {
        name: key.clone(),
        description: entry.description.clone().unwrap_or_else(|| key.clone()),
        behavior: entry.behavior.unwrap_or_else(|| default_behavior_for(&key)),
        model_alias: entry.model.clone().filter(|s| !s.trim().is_empty()),
        temperature: entry.temperature,
        prompt_extra: entry.prompt_file.as_deref().and_then(load_prompt_extra),
        tools,
    })
}

/// Sorted mode names, builtins (norm/plan/ask) first.
pub fn mode_names() -> Vec<String> {
    let cfg = load().unwrap_or_else(|_| template_config());
    let mut customs: Vec<String> = cfg
        .modes
        .keys()
        .filter(|k| !["norm", "plan", "ask"].contains(&k.as_str()))
        .cloned()
        .collect();
    customs.sort();
    let mut out = vec!["norm".to_string(), "plan".to_string(), "ask".to_string()];
    out.extend(customs.into_iter().filter(|n| n != "auto"));
    out
}

/// Effective access for a tool under a resolved mode. Unlisted tools default
/// to Allow; MCP `server__tool` names consult the `mcp` key.
pub fn tool_access(resolved: &ResolvedMode, tool: &str) -> ToolAccess {
    if tool.contains("__") {
        return resolved
            .tools
            .get(MCP_TOOL_KEY)
            .copied()
            .unwrap_or(ToolAccess::Allow);
    }
    resolved
        .tools
        .get(tool)
        .copied()
        .unwrap_or(ToolAccess::Allow)
}

/// Drop tool definitions the mode denies. Operates on chat-shaped defs
/// (`type/function/name`); use the existing responses converter after.
/// ReadOnly/PlansOnly tools stay visible — the execution gate enforces the
/// per-invocation restriction. Returns the filtered vec.
pub fn filter_definitions(
    defs: Vec<serde_json::Value>,
    resolved: &ResolvedMode,
) -> Vec<serde_json::Value> {
    defs.into_iter()
        .filter(|v| {
            let name = v
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            // ask_user is the approval channel itself — never filtered.
            if name == "ask_user" {
                return true;
            }
            tool_access(resolved, name) != ToolAccess::Deny
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_access_parses_bool_and_names() {
        let b: RawAccess = serde_json::from_str("true").unwrap();
        assert_eq!(ToolAccess::from_raw(&b), ToolAccess::Allow);
        let b: RawAccess = serde_json::from_str("false").unwrap();
        assert_eq!(ToolAccess::from_raw(&b), ToolAccess::Deny);
        let b: RawAccess = serde_json::from_str("\"plans-only\"").unwrap();
        assert_eq!(ToolAccess::from_raw(&b), ToolAccess::PlansOnly);
        let b: RawAccess = serde_json::from_str("\"readonly\"").unwrap();
        assert_eq!(ToolAccess::from_raw(&b), ToolAccess::ReadOnly);
        let b: RawAccess = serde_json::from_str("\"read-only\"").unwrap();
        assert_eq!(ToolAccess::from_raw(&b), ToolAccess::ReadOnly);
    }

    #[test]
    fn builtin_plan_restricts_writes() {
        let cfg = template_config();
        let plan = &cfg.modes["plan"];
        let tools = plan.tools.as_ref().unwrap();
        assert_eq!(ToolAccess::from_raw(&tools["write"]), ToolAccess::PlansOnly);
        assert_eq!(ToolAccess::from_raw(&tools["bash"]), ToolAccess::ReadOnly);
    }

    #[test]
    fn merge_entry_project_overrides_global() {
        let mut base = ModeEntry {
            temperature: Some(0.5),
            tools: Some(HashMap::from([("bash".to_string(), RawAccess::Bool(true))])),
            ..Default::default()
        };
        let over = ModeEntry {
            model: Some("gpt-4o".to_string()),
            tools: Some(HashMap::from([(
                "bash".to_string(),
                RawAccess::Bool(false),
            )])),
            ..Default::default()
        };
        merge_entry(&mut base, &over);
        assert_eq!(base.model.as_deref(), Some("gpt-4o"));
        assert_eq!(base.temperature, Some(0.5));
        assert_eq!(
            ToolAccess::from_raw(&base.tools.as_ref().unwrap()["bash"]),
            ToolAccess::Deny
        );
    }

    #[test]
    fn tool_access_routes_mcp_to_mcp_key() {
        let resolved = ResolvedMode {
            name: "plan".to_string(),
            description: String::new(),
            behavior: GateBehavior::Plan,
            model_alias: None,
            temperature: None,
            prompt_extra: None,
            tools: HashMap::from([(MCP_TOOL_KEY.to_string(), ToolAccess::ReadOnly)]),
        };
        assert_eq!(
            tool_access(&resolved, "github__create_issue"),
            ToolAccess::ReadOnly
        );
        assert_eq!(tool_access(&resolved, "read"), ToolAccess::Allow);
    }

    #[test]
    fn filter_definitions_drops_denied_keeps_ask_user() {
        let resolved = ResolvedMode {
            name: "ask".to_string(),
            description: String::new(),
            behavior: GateBehavior::Ask,
            model_alias: None,
            temperature: None,
            prompt_extra: None,
            tools: HashMap::from([
                ("write".to_string(), ToolAccess::Deny),
                ("bash".to_string(), ToolAccess::ReadOnly),
            ]),
        };
        let defs = vec![
            serde_json::json!({"type":"function","function":{"name":"write"}}),
            serde_json::json!({"type":"function","function":{"name":"bash"}}),
            serde_json::json!({"type":"function","function":{"name":"ask_user"}}),
        ];
        let out = filter_definitions(defs, &resolved);
        let names: Vec<_> = out
            .iter()
            .map(|v| v["function"]["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["bash".to_string(), "ask_user".to_string()]);
    }
}
