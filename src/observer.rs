use serde_json::{json, Value};

const OBSERVER_SYSTEM: &str = r#"You are the observation agent for a coding assistant.

Your job: compress ONE chunk of recent conversation into atomic observations.
Each observation is ONE plain sentence, single line, no markdown, no bullets.
Preserve user assertions exactly (User stated...), questions as questions, completions as "completed:".
Use precise verbs (installed, subscribed, configured) and keep file paths, ticket ids, error codes verbatim.
Split compound facts into separate observations.

Return ONLY a JSON array of strings, e.g. ["User stated they have two kids.", "completed: implemented login at src/auth/login.ts"]
If chunk has no new information, return [].

Timestamp format not needed — lean adds it.
"#;

const OBSERVER_KICKOFF: &str = "Compress the following conversation chunk into observations (JSON array only):";

fn chunk_to_text(chunk: &str) -> String {
    chunk.chars().take(6000).collect()
}

pub async fn observe_chunk(chunk: String, model: String) {
    if chunk.trim().len() < 40 {
        return;
    }
    let client = crate::llm::Client::from_env();
    let use_model = if model.is_empty() {
        "mimo-v2.5-free".to_string()
    } else {
        model
    };
    let body = json!({
        "model": use_model,
        "messages": [
            {"role": "system", "content": OBSERVER_SYSTEM},
            {"role": "user", "content": format!("{}\n\n---BEGIN CHUNK---\n{}\n---END CHUNK---", OBSERVER_KICKOFF, chunk_to_text(&chunk))}
        ],
        "temperature": 0.2,
        "max_tokens": 800
    });

    let resp = match client
        .http
        .post(client.chat_url())
        .header("Authorization", format!("Bearer {}", client.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return,
    };
    if !resp.status().is_success() {
        return;
    }
    let json: Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return,
    };
    let content = json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let trimmed = content.trim();
    let arr: Vec<Value> = if let Ok(v) = serde_json::from_str::<Vec<Value>>(trimmed) {
        v
    } else if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            let slice = &trimmed[start..=end];
            serde_json::from_str(slice).unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    for item in arr {
        if let Some(s) = item.as_str() {
            let obs = s.trim();
            if obs.is_empty() || obs.len() < 8 {
                continue;
            }
            if obs.starts_with('-') || obs.starts_with('*') || obs.starts_with('#') {
                continue;
            }
            crate::memory::api_remember(
                obs,
                "fact",
                vec!["observer".to_string(), "auto".to_string()],
                "global",
            );
        }
    }
}

pub fn build_chunk_text(roles_contents: &[(String, String)]) -> String {
    let mut out = String::new();
    for (role, content) in roles_contents {
        out.push_str(&format!("[{}] {}\n", role, content));
    }
    out
}
