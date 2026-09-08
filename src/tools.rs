use base64::Engine;
use std::path::Path;

const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;

#[derive(Debug, Clone, Copy)]
pub enum TruncateStrategy {
    Head,
    Tail,
}

pub fn truncate_output(content: &str, strategy: TruncateStrategy) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut truncated = content.to_string();
    let mut was_truncated = false;
    let mut kept_lines = lines.len();
    let mut kept_bytes = content.len();

    // Apply line limit first
    if lines.len() > MAX_LINES {
        was_truncated = true;
        match strategy {
            TruncateStrategy::Head => {
                let kept = &lines[..MAX_LINES];
                truncated = kept.join("\n");
                kept_lines = MAX_LINES;
                kept_bytes = truncated.len();
            }
            TruncateStrategy::Tail => {
                let kept = &lines[lines.len() - MAX_LINES..];
                truncated = kept.join("\n");
                kept_lines = MAX_LINES;
                kept_bytes = truncated.len();
            }
        }
    }

    // Apply byte limit (never split mid-line — adjust to line boundary)
    if truncated.len() > MAX_BYTES {
        was_truncated = true;
        match strategy {
            TruncateStrategy::Head => {
                // keep from start, cut at last newline before MAX_BYTES
                let mut cut = MAX_BYTES;
                while cut > 0 && !truncated.is_char_boundary(cut) {
                    cut -= 1;
                }
                let mut s = truncated[..cut].to_string();
                if let Some(pos) = s.rfind('\n') {
                    s.truncate(pos);
                }
                truncated = s;
                kept_bytes = truncated.len();
                kept_lines = truncated.lines().count();
            }
            TruncateStrategy::Tail => {
                let start = truncated.len().saturating_sub(MAX_BYTES);
                let mut cut = start;
                while cut < truncated.len() && !truncated.is_char_boundary(cut) {
                    cut += 1;
                }
                let mut s = truncated[cut..].to_string();
                if let Some(pos) = s.find('\n') {
                    s = s[pos + 1..].to_string();
                }
                truncated = s;
                kept_bytes = truncated.len();
                kept_lines = truncated.lines().count();
            }
        }
    }

    if was_truncated {
        let note = format!(
            "\n… [truncated: kept {} lines / {} bytes of {} lines / {} bytes]",
            kept_lines,
            kept_bytes,
            content.lines().count(),
            content.len()
        );
        truncated.push_str(&note);
    }
    truncated
}

pub async fn read_file(path: &str) -> Result<String, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("File not found: {}", path));
    }
    if is_image_file(path) {
        return read_image(path).await;
    }
    let raw = tokio::fs::read_to_string(p)
        .await
        .map_err(|e| format!("read error: {}", e))?;
    Ok(truncate_output(&raw, TruncateStrategy::Head))
}

/// Return the MIME type for a file extension, if it's a known image format.
fn image_mime_type(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        "ico" => Some("image/x-icon"),
        _ => None,
    }
}

/// Check if a file path looks like an image.
fn is_image_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(image_mime_type)
        .is_some()
}

/// Read an image file and return its base64-encoded content.
/// The returned string contains the special marker `<<IMAGE:mime:base64>>`
/// which agent.rs uses to construct a multimodal content message.
async fn read_image(path: &str) -> Result<String, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("File not found: {}", path));
    }
    let ext = p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = image_mime_type(&ext).unwrap_or("image/png");
    let bytes = tokio::fs::read(p)
        .await
        .map_err(|e| format!("read error: {}", e))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!(
        "Read image: {} ({}. {})\n<<IMAGE:{}:{}>>",
        path,
        mime,
        human_size(bytes.len()),
        mime,
        encoded
    ))
}

fn human_size(bytes: usize) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

pub async fn write_file(path: &str, content: &str) -> Result<String, String> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir error: {}", e))?;
        }
    }
    tokio::fs::write(p, content)
        .await
        .map_err(|e| format!("write error: {}", e))?;
    Ok(format!("Wrote {} bytes to {}", content.len(), path))
}

pub async fn edit_file(path: &str, old: &str, new: &str) -> Result<String, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("File not found: {}", path));
    }
    let raw = tokio::fs::read_to_string(p)
        .await
        .map_err(|e| format!("read error: {}", e))?;
    let count = raw.matches(old).count();
    if count == 0 {
        return Err("oldText not found (no match)".to_string());
    }
    if count > 1 {
        return Err(format!(
            "oldText matched {} times — must be unique",
            count
        ));
    }
    let updated = raw.replacen(old, new, 1);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir error: {}", e))?;
        }
    }
    tokio::fs::write(p, updated)
        .await
        .map_err(|e| format!("write error: {}", e))?;
    Ok(format!("Edited {}", path))
}

pub async fn run_bash(command: &str) -> Result<String, String> {
    let out = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .output()
        .await
        .map_err(|e| format!("spawn error: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let code = out.status.code().unwrap_or(-1);
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&format!("[stderr]\n{}", stderr));
    }
    if combined.is_empty() {
        combined.push_str(&format!("[exit code {}]", code));
    } else {
        combined.push_str(&format!("\n[exit code {}]", code));
    }
    Ok(truncate_output(&combined, TruncateStrategy::Tail))
}

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
    let url = format!("https://api.duckduckgo.com/?q={}&format=json&no_html=1", query);
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

pub async fn execute_tool(name: &str, args: serde_json::Value) -> String {
    let res = match name {
        "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            read_file(path).await
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            write_file(path, content).await
        }
        "edit_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let old = args.get("oldText").and_then(|v| v.as_str()).unwrap_or("");
            let new = args.get("newText").and_then(|v| v.as_str()).unwrap_or("");
            // also support snake_case fallback
            let old2 = if old.is_empty() {
                args.get("old_text").and_then(|v| v.as_str()).unwrap_or("")
            } else {
                old
            };
            let new2 = if new.is_empty() {
                args.get("new_text").and_then(|v| v.as_str()).unwrap_or("")
            } else {
                new
            };
            edit_file(path, old2, new2).await
        }
        "bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            run_bash(cmd).await
        }
        "web_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            web_search(q).await
        }
        "read_skill" => {
            let n = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match crate::skills::load_skill(n).await {
                Ok(c) => Ok(c),
                Err(e) => Err(format!("{}", e)),
            }
        }
        "remember" => {
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("fact");
            let tags: Vec<String> = args.get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("global");
            Ok(crate::memory::api_remember(content, category, tags, scope))
        }
        "search_memory" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            Ok(crate::memory::api_search(q))
        }
        "recall_memory" => {
            Ok(crate::memory::api_recall())
        }
        "list_memories" => {
            let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            Ok(crate::memory::api_list(tag))
        }
        "forget_memory" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            Ok(crate::memory::api_forget(id))
        }
        "todo" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            match action {
                "add" => {
                    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let priority = args.get("priority").and_then(|v| v.as_str()).unwrap_or("medium");
                    let group = args.get("group").and_then(|v| v.as_str()).unwrap_or("");
                    Ok(crate::todo::api_add(content, priority, group))
                }
                "update" => {
                    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let status = args.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    Ok(crate::todo::api_update(id, status))
                }
                "remove" => {
                    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    Ok(crate::todo::api_remove(id))
                }
                _ => Ok(crate::todo::api_list()),
            }
        }
        _ => Err(format!("unknown tool: {}", name)),
    };
    match res {
        Ok(s) => s,
        Err(e) => format!("Error: {}", e),
    }
}
