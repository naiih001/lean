use std::sync::OnceLock;

pub const DEFAULT_MODEL: &str = "mimo-v2.5-free";

#[derive(Debug, Clone)]
pub struct Client {
    pub api_key: String,
    pub base_url: String,
    pub http: reqwest::Client,
    pub provider: crate::integrations::models::Provider,
    pub api_mode: crate::integrations::models::ApiMode,
}

impl Client {
    pub fn from_env() -> Self {
        let api_key = std::env::var("OPENCODE_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .unwrap_or_else(|_| "sk-test".to_string());
        let base_url = std::env::var("OPENCODE_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080/zen/v1".to_string());
        Self {
            api_key,
            base_url,
            http: reqwest::Client::builder()
                .user_agent("opencode/1.0")
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            provider: crate::integrations::models::Provider::Generic,
            api_mode: crate::integrations::models::ApiMode::ChatCompletions,
        }
    }

    pub fn from_resolved(resolved: &crate::integrations::models::ResolvedModel) -> Self {
        let timeout_secs = match resolved.provider {
            crate::integrations::models::Provider::Ollama => 30,
            _ => 60,
        };
        Self {
            api_key: resolved.api_key.clone(),
            base_url: resolved.base_url.clone(),
            http: reqwest::Client::builder()
                .user_agent("opencode/1.0")
                .timeout(std::time::Duration::from_secs(timeout_secs))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            provider: resolved.provider.clone(),
            api_mode: resolved.api_mode.clone(),
        }
    }

    fn opencode_session_id() -> String {
        static SESSION: OnceLock<String> = OnceLock::new();
        SESSION
            .get_or_init(|| {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                format!("lean-{:012x}", nanos & 0xffffffffff)
            })
            .clone()
    }

    pub fn is_opencode(&self) -> bool {
        self.base_url.contains("opencode.ai")
            || matches!(
                self.provider,
                crate::integrations::models::Provider::Generic
            ) && self.base_url.contains("opencode")
    }

    pub fn opencode_headers(&self) -> Vec<(String, String)> {
        if !self.is_opencode() {
            return Vec::new();
        }
        vec![
            (
                "x-opencode-session".to_string(),
                Self::opencode_session_id(),
            ),
            ("x-opencode-client".to_string(), "pi".to_string()),
        ]
    }

    pub fn is_anthropic(&self) -> bool {
        let is_anthropic_api = self.api_mode == crate::integrations::models::ApiMode::Anthropic;
        let is_anthropic_provider =
            self.provider == crate::integrations::models::Provider::Anthropic;
        let base_is_anthropic = self.base_url.contains("api.anthropic.com");
        (is_anthropic_api || is_anthropic_provider) && base_is_anthropic
    }

    pub fn chat_url(&self) -> String {
        if self.is_anthropic() {
            let base = self.base_url.trim_end_matches('/');
            if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            }
        } else {
            format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
        }
    }

    pub fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }

    pub fn anthropic_url(&self) -> String {
        self.chat_url()
    }

    pub fn auth_headers(&self) -> Vec<(String, String)> {
        if self.is_anthropic() {
            vec![
                ("x-api-key".to_string(), self.api_key.clone()),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ]
        } else {
            vec![(
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            )]
        }
    }

    pub fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut b = builder;
        for (k, v) in self.auth_headers() {
            b = b.header(k, v);
        }
        for (k, v) in self.opencode_headers() {
            b = b.header(k, v);
        }
        b
    }
}
