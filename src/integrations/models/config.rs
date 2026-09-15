use crate::integrations::models::provider::{ApiMode, Provider};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ApiMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
}

impl ModelEntry {
    pub fn api_mode(&self) -> ApiMode {
        if let Some(m) = self.api.clone() {
            return m;
        }
        if self.provider() == Provider::Anthropic {
            return ApiMode::Anthropic;
        }
        ApiMode::default()
    }
    pub fn provider(&self) -> Provider {
        if let Some(p) = self.provider.clone() {
            return p;
        }
        if let Some(url) = &self.base_url {
            let lower = url.to_lowercase();
            if lower.contains("api.anthropic.com") {
                return Provider::Anthropic;
            }
            if lower.contains("localhost:11434")
                || lower.contains("127.0.0.1:11434")
                || lower.contains("localhost:1234")
                || lower.contains("127.0.0.1:1234")
            {
                return Provider::Ollama;
            }
        }
        if self.api == Some(ApiMode::Anthropic) {
            return Provider::Anthropic;
        }
        Provider::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    pub default: String,
    pub models: HashMap<String, ModelEntry>,
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub alias: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub api_mode: ApiMode,
    pub provider: Provider,
    pub vision: bool,
}

pub(crate) fn models_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("models.json")
}

pub fn default_env_base_url() -> String {
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

pub fn ollama_base_url_from_env() -> String {
    let raw = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string());
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "http://localhost:11434/v1".to_string();
    }
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{}/v1", trimmed)
    }
}

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
    ModelsConfig {
        default: "gpt-4o".to_string(),
        models,
    }
}

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

pub fn load() -> Result<ModelsConfig> {
    let path = models_path();
    if !path.exists() {
        return ensure_exists();
    }
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cfg: ModelsConfig = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parse {} as JSON (check syntax with `jq empty {}`)",
            path.display(),
            path.display()
        )
    })?;
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
    let base_url = entry
        .base_url
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| match provider {
            Provider::Ollama => Provider::Ollama.default_base_url(),
            Provider::Anthropic => Provider::Anthropic.default_base_url(),
            _ => default_env_base_url(),
        });
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
    if provider == Provider::Ollama {
        if let Some(env_name) = &entry.api_key_env {
            if !env_name.trim().is_empty() {
                if let Ok(v) = std::env::var(env_name) {
                    if !v.trim().is_empty() {
                        return Ok(v);
                    }
                }
            }
        }
        return Ok("ollama".to_string());
    }
    if provider == Provider::Anthropic {
        if let Some(env_name) = &entry.api_key_env {
            if !env_name.trim().is_empty() {
                if let Ok(v) = std::env::var(env_name) {
                    if !v.trim().is_empty() {
                        return Ok(v);
                    }
                }
                let fallback = default_anthropic_api_key();
                if fallback != "sk-test" && !fallback.trim().is_empty() {
                    return Ok(fallback);
                }
                bail!(
                    "No API key found for '{}' — set {} or add `api_key` to {}",
                    env_name,
                    env_name,
                    models_path().display()
                );
            }
        }
        let fallback = default_anthropic_api_key();
        if fallback != "sk-test" && !fallback.trim().is_empty() {
            return Ok(fallback);
        }
        bail!("No API key found for 'ANTHROPIC_API_KEY' — set ANTHROPIC_API_KEY or add `api_key` to {}", models_path().display());
    }
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
        assert!(!cfg.models.values().any(|e| e
            .base_url
            .as_deref()
            .unwrap_or("")
            .contains("127.0.0.1")));
        let e = &cfg.models["gpt-4o"];
        assert_eq!(e.base_url.as_deref(), Some("https://api.openai.com/v1"));
    }
    #[test]
    fn resolve_fails_without_key() {
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
        if let Some(v) = orig_openai {
            std::env::set_var("OPENAI_API_KEY", v);
        }
        if let Some(v) = orig_opencode {
            std::env::set_var("OPENCODE_API_KEY", v);
        }
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
        let entry = ModelEntry {
            provider: Some(Provider::Ollama),
            model: "qwen2".into(),
            base_url: None,
            api_key: None,
            api_key_env: None,
            api: None,
            vision: None,
        };
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
