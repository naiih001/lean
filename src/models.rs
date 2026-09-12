use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Which OpenAI-compatible wire format to use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiMode {
    #[serde(alias = "chat", alias = "chat_completions", alias = "completions")]
    ChatCompletions,
    #[serde(alias = "responses")]
    Responses,
    #[serde(alias = "anthropic")]
    Anthropic,
}

impl Default for ApiMode {
    fn default() -> Self {
        Self::ChatCompletions
    }
}

/// Provider — determines base_url defaults, auth headers, and wire format.
/// `provider` is explicit in models.json; when missing we infer from base_url for backward compat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[serde(alias = "openai")]
    OpenAI,
    #[serde(alias = "anthropic")]
    Anthropic,
    #[serde(alias = "ollama", alias = "local", alias = "lmstudio")]
    Ollama,
    #[serde(alias = "generic", alias = "opencode", alias = "agentrouter")]
    Generic,
}

impl Default for Provider {
    fn default() -> Self { Self::OpenAI }
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::Generic => "generic",
        }
    }
    pub fn requires_key(&self) -> bool {
        match self {
            Self::Ollama => false,
            _ => true,
        }
    }
    pub fn default_base_url(&self) -> String {
        match self {
            Self::OpenAI => "https://api.openai.com/v1".to_string(),
            Self::Anthropic => "https://api.anthropic.com".to_string(),
            Self::Ollama => "http://localhost:11434/v1".to_string(),
            Self::Generic => default_env_base_url(),
        }
    }
    pub fn default_key_env(&self) -> Option<&'static str> {
        match self {
            Self::OpenAI => Some("OPENAI_API_KEY"),
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::Ollama => None,
            Self::Generic => None,
        }
    }
}

/// Entry for a single model alias.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Real model id sent to the API (e.g. "gpt-4o").
    pub model: String,
    /// Provider — openai (default), anthropic, ollama/local, generic.
    /// When missing, inferred from base_url for backward compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// Optional base_url override (e.g. "https://api.openai.com/v1").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Inline API key (not recommended, but supported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Env var name that holds the API key (e.g. "OPENAI_API_KEY").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Wire format: "chat_completions" (default) or "responses" / "anthropic".
    /// Aliases: "chat"/"completions" -> chat, "responses" -> responses, "anthropic" -> anthropic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ApiMode>,
    /// Whether this model supports multimodal (vision) input.
    /// When missing, defaults to true. Set to false for text-only models
    /// to prevent sending images that trigger API errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
}

impl ModelEntry {
    pub fn api_mode(&self) -> ApiMode {
        if let Some(m) = self.api.clone() { return m; }
        // If provider is anthropic and no explicit api, default to Anthropic wire format
        if self.provider() == Provider::Anthropic { return ApiMode::Anthropic; }
        ApiMode::default()
    }
    pub fn provider(&self) -> Provider {
        if let Some(p) = self.provider.clone() { return p; }
        // Infer for backward compat: old configs without provider field
        if let Some(url) = &self.base_url {
            let lower = url.to_lowercase();
            if lower.contains("api.anthropic.com") { return Provider::Anthropic; }
            if lower.contains("localhost:11434") || lower.contains("127.0.0.1:11434") || lower.contains("localhost:1234") || lower.contains("127.0.0.1:1234") { return Provider::Ollama; }
        }
        // Also check api field
        if self.api == Some(ApiMode::Anthropic) { return Provider::Anthropic; }
        Provider::default()
    }
}

/// Top-level config stored at ~/.lean/models.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    pub default: String,
    pub models: HashMap<String, ModelEntry>,
}

/// Resolved model — alias + concrete provider details ready for llm::Client.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub alias: String,
    /// Real model id to send in `model` field.
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub api_mode: ApiMode,
    pub provider: Provider,
    /// Whether the model supports vision/multimodal input (defaults to true).
    pub vision: bool,
}

fn models_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("models.json")
}

fn default_env_base_url() -> String {
    std::env::var("OPENCODE_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080/zen/v1".to_string())
}

fn default_env_api_key() -> String {
    std::env::var("OPENCODE_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_else(|_| "sk-test".to_string())
}

fn default_anthropic_api_key() -> String {
    std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".to_string())
}

/// Check if Ollama is reachable at localhost:11434 and return model names. Sync best-effort with timeout.
fn ollama_detect_sync() -> Vec<String> {
    // Use blocking reqwest with short timeout to avoid hanging
    let try_client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_millis(400)).build();
    if let Ok(client) = try_client {
        if let Ok(resp) = client.get("http://localhost:11434/api/tags").send() {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                    return models.iter().filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect();
                }
            }
        }
        // Also try 127.0.0.1:11434 as fallback (same, but be explicit)
        if let Ok(resp) = client.get("http://127.0.0.1:11434/api/tags").send() {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                    return models.iter().filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect();
                }
            }
        }
    }
    Vec::new()
}

pub fn ollama_available() -> bool { !ollama_detect_sync().is_empty() }

pub fn ollama_model_names() -> Vec<String> { ollama_detect_sync() }

/// Canonical fresh-install template — must stay OpenAI (Q3). Do not change to localhost zen proxy.
fn template_config() -> ModelsConfig {
    let mut models = HashMap::new();
    models.insert(
        "gpt-4o".to_string(),
        ModelEntry {
            model: "gpt-4o".to_string(),
            provider: Some(Provider::OpenAI),
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            api: None,
            vision: None,
        },
    );
    // Examples are intentionally NOT added to fresh template — they are documented in README and can be added via `lean --provider` helpers.
    // Keeping template minimal ensures Q3 "must stay OpenAI" check passes.
    ModelsConfig {
        default: "gpt-4o".to_string(),
        models,
    }
}

fn example_ollama_entry(model: &str) -> ModelEntry {
    ModelEntry { model: model.to_string(), provider: Some(Provider::Ollama), base_url: Some(Provider::Ollama.default_base_url()), api_key: Some("ollama".to_string()), api_key_env: None, api: None, vision: None }
}

fn example_anthropic_entry(model: &str) -> ModelEntry {
    ModelEntry { model: model.to_string(), provider: Some(Provider::Anthropic), base_url: Some(Provider::Anthropic.default_base_url()), api_key: None, api_key_env: Some("ANTHROPIC_API_KEY".to_string()), api: Some(ApiMode::Anthropic), vision: None }
}

/// Ensure ~/.lean/models.json exists, creating a template if missing.
/// Returns the loaded config.
pub fn ensure_exists() -> Result<ModelsConfig> {
    let path = models_path();
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let cfg = template_config();
        let json = serde_json::to_string_pretty(&cfg).context("serialize template")?;
        std::fs::write(&path, json)
            .with_context(|| format!("write template {}", path.display()))?;
        return Ok(cfg);
    }
    load()
}

/// Load and parse the config. If file doesn't exist, create template first.
pub fn load() -> Result<ModelsConfig> {
    let path = models_path();
    if !path.exists() {
        return ensure_exists();
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let cfg: ModelsConfig = serde_json::from_str(&raw)
        .with_context(|| format!("parse {} as JSON (check syntax with `jq empty {}`)", path.display(), path.display()))?;
    if cfg.models.is_empty() {
        bail!("{} has no models defined", path.display());
    }
    if !cfg.models.contains_key(&cfg.default) {
        bail!(
            "{}: default '{}' not found in models (available: {})",
            path.display(),
            cfg.default,
            cfg.models.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    Ok(cfg)
}

/// Resolve an alias (or None → default) to a concrete ResolvedModel with env fallbacks.
pub fn resolve(alias_opt: Option<&str>) -> Result<ResolvedModel> {
    let cfg = load()?;
    let alias = alias_opt
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.default.clone());
    let alias_trimmed = alias.trim();
    if alias_trimmed.is_empty() {
        bail!("empty model alias");
    }
    let entry = cfg.models.get(alias_trimmed).ok_or_else(|| {
        let available = cfg.models.keys().cloned().collect::<Vec<_>>().join(", ");
        anyhow::anyhow!(
            "unknown model alias '{}' (available: {}) — edit {}",
            alias_trimmed,
            available,
            models_path().display()
        )
    })?;

    let provider = entry.provider();
    // base_url: entry override → provider default → env → default proxy
    let base_url = entry
        .base_url
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            // Provider defaults take precedence over env for ollama/anthropic
            match provider {
                Provider::Ollama => Provider::Ollama.default_base_url(),
                Provider::Anthropic => Provider::Anthropic.default_base_url(),
                _ => default_env_base_url(),
            }
        });

    // api_key resolution: inline → env var ref → provider-aware fallback
    // For Ollama, no key required — return "ollama" placeholder if no env set
    let api_key = if let Some(k) = &entry.api_key {
        if !k.trim().is_empty() {
            k.clone()
        } else {
            resolve_api_key_env(entry, None)?
        }
    } else {
        resolve_api_key_env(entry, None)?
    };

    let api_mode = entry.api_mode();
    Ok(ResolvedModel {
        alias: alias_trimmed.to_string(),
        model: entry.model.clone(),
        base_url,
        api_key,
        api_mode,
        provider,
        vision: entry.vision.unwrap_or(true),
    })
}

fn resolve_api_key_env(entry: &ModelEntry, _alias: Option<&str>) -> Result<String> {
    let provider = entry.provider();
    // Ollama/local never requires a real key — return placeholder if no env set
    if provider == Provider::Ollama {
        // If inline key already handled, we are here because no inline. Try env if specified.
        if let Some(env_name) = &entry.api_key_env {
            if !env_name.trim().is_empty() {
                if let Ok(v) = std::env::var(env_name) {
                    if !v.trim().is_empty() { return Ok(v); }
                }
            }
        }
        // No key needed for Ollama — use dummy placeholder that will be sent as Bearer but ignored by Ollama
        return Ok("ollama".to_string());
    }
    // Anthropic: try ANTHROPIC_API_KEY specifically, then fallback
    if provider == Provider::Anthropic {
        if let Some(env_name) = &entry.api_key_env {
            if !env_name.trim().is_empty() {
                if let Ok(v) = std::env::var(env_name) {
                    if !v.trim().is_empty() { return Ok(v); }
                }
                // Try ANTHROPIC_API_KEY fallback before error
                let fallback = default_anthropic_api_key();
                if fallback != "sk-test" && !fallback.trim().is_empty() { return Ok(fallback); }
                bail!(
                    "No API key found for '{}' — set {} or add `api_key` to {}",
                    env_name, env_name, models_path().display()
                );
            }
        }
        let fallback = default_anthropic_api_key();
        if fallback != "sk-test" && !fallback.trim().is_empty() { return Ok(fallback); }
        bail!("No API key found for 'ANTHROPIC_API_KEY' — set ANTHROPIC_API_KEY or add `api_key` to {}", models_path().display());
    }
    // OpenAI / Generic: original logic
    let try_fallback = |env_name: Option<&String>| -> Result<String> {
        let fallback = default_env_api_key();
        if fallback == "sk-test" || fallback.trim().is_empty() {
            let hint_name = env_name.map(|s| s.as_str()).unwrap_or("OPENAI_API_KEY");
            bail!(
                "No API key found for '{}' — set {} (or OPENCODE_API_KEY) or add `api_key` to {}",
                hint_name,
                hint_name,
                models_path().display()
            );
        }
        Ok(fallback)
    };
    if let Some(env_name) = &entry.api_key_env {
        if !env_name.trim().is_empty() {
            if let Ok(v) = std::env::var(env_name) {
                if !v.trim().is_empty() {
                    return Ok(v);
                }
            }
            return try_fallback(Some(env_name));
        }
    }
    try_fallback(None)
}

/// List available aliases sorted.
pub fn list_aliases() -> Result<Vec<String>> {
    let cfg = load()?;
    let mut v: Vec<String> = cfg.models.keys().cloned().collect();
    v.sort();
    Ok(v)
}

/// Return the default alias.
pub fn default_alias() -> Result<String> {
    Ok(load()?.default)
}

/// Return the path for display/error messages.
pub fn path_display() -> String {
    models_path().display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_has_gpt4o() {
        let cfg = template_config();
        assert_eq!(cfg.default, "gpt-4o");
        assert!(cfg.models.contains_key("gpt-4o"));
        let e = &cfg.models["gpt-4o"];
        assert_eq!(e.model, "gpt-4o");
        assert_eq!(e.base_url.as_deref(), Some("https://api.openai.com/v1"));
        assert_eq!(e.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
    }

    #[test]
    fn template_is_openai_default() {
        let cfg = template_config();
        assert!(!cfg.default.contains("mimo"));
        assert!(!cfg.models.values().any(|e| e.base_url.as_deref().unwrap_or("").contains("127.0.0.1")));
        let e = &cfg.models["gpt-4o"];
        assert_eq!(e.base_url.as_deref(), Some("https://api.openai.com/v1"));
    }

    #[test]
    fn resolve_fails_without_key() {
        // Isolate env: save and clear both keys, use a temp home via models_path that still points to real path
        // We test resolve_api_key_env directly to avoid touching ~/.lean/models.json
        let orig_openai = std::env::var("OPENAI_API_KEY").ok();
        let orig_opencode = std::env::var("OPENCODE_API_KEY").ok();
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("OPENCODE_API_KEY");
        let entry = ModelEntry {
            provider: None,
            model: "gpt-4o".into(),
            base_url: Some("https://api.openai.com/v1".into()),
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".into()),
            api: None,
            vision: None,
        };
        let err = resolve_api_key_env(&entry, None).unwrap_err();
        assert!(err.to_string().contains("OPENAI_API_KEY"));
        // restore
        if let Some(v) = orig_openai { std::env::set_var("OPENAI_API_KEY", v); }
        if let Some(v) = orig_opencode { std::env::set_var("OPENCODE_API_KEY", v); }
    }

    #[test]
    fn provider_defaults_to_openai() {
        let raw = r#"{"model":"gpt-4o"}"#;
        let e: ModelEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(e.provider(), Provider::OpenAI);
        assert_eq!(e.api_mode(), ApiMode::ChatCompletions);
    }

    #[test]
    fn provider_ollama_no_key() {
        let entry = ModelEntry { provider: Some(Provider::Ollama), model: "qwen2".into(), base_url: None, api_key: None, api_key_env: None, api: None, vision: None };
        // Ollama should not fail even without env keys
        let key = resolve_api_key_env(&entry, None).unwrap();
        assert_eq!(key, "ollama");
        assert_eq!(entry.provider(), Provider::Ollama);
    }

    #[test]
    fn model_entry_defaults_to_chat() {
        let raw = r#"{"model":"gpt-4o","base_url":"https://api.openai.com/v1"}"#;
        let e: ModelEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(e.api_mode(), ApiMode::ChatCompletions);
    }

    #[test]
    fn model_entry_parses_responses_alias() {
        let raw = r#"{"model":"gpt-5","api":"responses"}"#;
        let e: ModelEntry = serde_json::from_str(raw).unwrap();
        assert_eq!(e.api_mode(), ApiMode::Responses);
    }

    #[test]
    fn model_entry_parses_chat_aliases() {
        for alias in ["chat", "chat_completions", "completions"] {
            let raw = format!(r#"{{"model":"gpt-4o","api":"{}"}}"#, alias);
            let e: ModelEntry = serde_json::from_str(&raw).unwrap();
            assert_eq!(e.api_mode(), ApiMode::ChatCompletions, "alias {}", alias);
        }
    }
}
