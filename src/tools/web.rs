use crate::tools::fs::{truncate_output, TruncateStrategy};

pub async fn web_search(query: &str) -> Result<String, String> {
    // Try Exa if key present, else DuckDuckGo
    if let Ok(exa_key) = std::env::var("EXA_API_KEY") {
        if !exa_key.is_empty() {
            let client = reqwest::Client::new();
            let body = serde_json::json!({
                "query": query,
                "numResults": 5
            });
            if let Ok(resp) = client
                .post("https://api.exa.ai/search")
                .header("x-api-key", exa_key)
                .json(&body)
                .send()
                .await
            {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    return Ok(truncate_output(
                        &serde_json::to_string_pretty(&json).unwrap_or_default(),
                        TruncateStrategy::Head,
                    ));
                }
            }
        }
    }
    // DuckDuckGo fallback
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1",
        query
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("search error: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("search read error: {}", e))?;
    Ok(truncate_output(&text, TruncateStrategy::Head))
}

pub async fn web_fetch(url: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("fetch error: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("fetch read error: {}", e))?;
    Ok(truncate_output(&text, TruncateStrategy::Head))
}
