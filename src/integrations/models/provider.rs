use serde::{Deserialize, Serialize};

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
    fn default() -> Self {
        Self::OpenAI
    }
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
    pub fn default_base_url(&self) -> String {
        match self {
            Self::OpenAI => "https://api.openai.com/v1".to_string(),
            Self::Anthropic => "https://api.anthropic.com".to_string(),
            Self::Ollama => crate::integrations::models::config::ollama_base_url_from_env(),
            Self::Generic => crate::integrations::models::config::default_env_base_url(),
        }
    }
}
