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

    /// Make a chat completion request. Used by the harness.
    pub async fn chat(
        &self,
        model: &str,
        messages: &[serde_json::Value],
        temperature: f32,
        max_tokens: Option<usize>,
    ) -> anyhow::Result<String> {
        let url = self.responses_url();
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens.unwrap_or(2000),
        });

        let policy = crate::integrations::llm::RetryPolicy::for_provider(&self.provider);
        let mut last_err: Option<String> = None;
        for attempt in 0..=policy.max_retries {
            let res = self
                .apply_auth(self.http.post(url.clone()))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;
            match res {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(resp.text().await.unwrap_or_default());
                    }
                    let status = resp.status().as_u16();
                    if is_retryable_status(status) && attempt < policy.max_retries {
                        let retry_after = parse_retry_after(resp.headers())
                            .unwrap_or(std::time::Duration::from_millis(policy.base_delays_ms[attempt as usize]));
                        let delay = std::cmp::min(retry_after, std::time::Duration::from_secs(30));
                        tokio::time::sleep(delay).await;
                        continue;
                    } else {
                        let txt = resp.text().await.unwrap_or_default();
                        last_err = Some(format!("HTTP {}: {}", status, txt));
                        break;
                    }
                }
                Err(e) => {
                    if is_retryable_error(&e) && attempt < policy.max_retries {
                        let delay = std::time::Duration::from_millis(policy.base_delays_ms[attempt as usize]);
                        tokio::time::sleep(delay).await;
                        continue;
                    } else {
                        last_err = Some(e.to_string());
                        break;
                    }
                }
            }
        }
        Err(anyhow::anyhow!("LLM call failed: {}", last_err.unwrap_or_else(|| "unknown error".to_string())))
    }
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500..=599)
}

fn is_retryable_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request() && format!("{:?}", e).to_lowercase().contains("connection")
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    if let Some(v) = headers.get(reqwest::header::RETRY_AFTER) {
        if let Ok(s) = v.to_str() {
            if let Ok(secs) = s.trim().parse::<u64>() {
                return Some(std::time::Duration::from_secs(secs));
            }
        }
    }
    None
}
