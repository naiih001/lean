use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Entry for a single model alias.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Real model id sent to the API (e.g. "gpt-4o").
    pub model: String,
    /// Optional base_url override (e.g. "https://api.openai.com/v1").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Inline API key (not recommended, but supported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Env var name that holds the API key (e.g. "OPENAI_API_KEY").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
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

/// Canonical fresh-install template — must stay OpenAI (Q3). Do not change to localhost zen proxy.
fn template_config() -> ModelsConfig {
    let mut models = HashMap::new();
    models.insert(
        "gpt-4o".to_string(),
        ModelEntry {
            model: "gpt-4o".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: None,
            api_key_env: Some("OPENAI_API_KEY".to_string()),
        },
    );
    ModelsConfig {
        default: "gpt-4o".to_string(),
        models,
    }
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

    // base_url: entry override → env → default proxy
    let base_url = entry
        .base_url
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(default_env_base_url);

    // api_key resolution: inline → env var ref → env fallback
    let api_key = if let Some(k) = &entry.api_key {
        if !k.trim().is_empty() {
            k.clone()
        } else {
            resolve_api_key_env(entry, None)?
        }
    } else {
        resolve_api_key_env(entry, None)?
    };

    Ok(ResolvedModel {
        alias: alias_trimmed.to_string(),
        model: entry.model.clone(),
        base_url,
        api_key,
    })
}

fn resolve_api_key_env(entry: &ModelEntry, _alias: Option<&str>) -> Result<String> {
    if let Some(env_name) = &entry.api_key_env {
        if !env_name.trim().is_empty() {
            if let Ok(v) = std::env::var(env_name) {
                if !v.trim().is_empty() {
                    return Ok(v);
                }
            }
            // env var not set or empty → fall back to generic env
            return Ok(default_env_api_key());
        }
    }
    Ok(default_env_api_key())
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
}
