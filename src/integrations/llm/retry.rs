#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub base_delays_ms: Vec<u64>,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delays_ms: vec![500, 1000, 2000],
        }
    }
}
impl RetryPolicy {
    pub fn ollama() -> Self {
        Self {
            max_retries: 2,
            base_delays_ms: vec![300, 600],
        }
    }
    pub fn for_provider(provider: &crate::models::Provider) -> Self {
        match provider {
            crate::models::Provider::Ollama => Self::ollama(),
            _ => Self::default(),
        }
    }
}
