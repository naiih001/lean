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
pub fn image_mime_type(ext: &str) -> Option<&'static str> {
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
pub fn is_image_file(path: &str) -> bool {
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
    let ext = p
        .extension()
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

pub fn human_size(bytes: usize) -> String {
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
        return Err(format!("oldText matched {} times — must be unique", count));
    }
    let updated = raw.replacen(old, new, 1);
    // Compute unified diff for native feel (like pi diff_mode)
    let diff = {
        use similar::TextDiff;
        let diff = TextDiff::from_lines(&raw, &updated);
        let unified = diff
            .unified_diff()
            .context_radius(3)
            .header("before", "after")
            .to_string();
        let out = format!("Edited {} — diff:\n{}", path, unified);
        let lines: Vec<&str> = out.lines().collect();
        if lines.len() > 120 {
            let kept = &lines[..120];
            format!(
                "{}\n… [diff truncated {} lines]",
                kept.join("\n"),
                lines.len() - 120
            )
        } else if unified.trim().is_empty() {
            format!(
                "Edited {} ({} bytes -> {} bytes)",
                path,
                raw.len(),
                updated.len()
            )
        } else {
            out
        }
    };
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
    Ok(diff)
}
