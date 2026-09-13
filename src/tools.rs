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
pub(crate) fn image_mime_type(ext: &str) -> Option<&'static str> {
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
pub(crate) fn is_image_file(path: &str) -> bool {
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

pub(crate) fn human_size(bytes: usize) -> String {
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
        let mut out = format!("Edited {} — diff:\n{}", path, unified);
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

fn contains_sudo(cmd: &str) -> bool {
    // token-aware check for sudo as command
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    tokens.iter().any(|t| *t == "sudo") || cmd.contains("sudo ")
}

fn inject_sudo_s(cmd: &str) -> String {
    // Insert -S -p '' after each sudo not already using -S
    let mut out = String::new();
    let mut chars = cmd.chars().peekable();
    let mut i = 0;
    let bytes: Vec<char> = cmd.chars().collect();
    while i < bytes.len() {
        if i + 4 <= bytes.len() && bytes[i..i + 4].iter().collect::<String>() == "sudo" {
            let prev_ok = i == 0 || bytes[i - 1].is_whitespace() || " ;|&(".contains(bytes[i - 1]);
            let next_ok = i + 4 == bytes.len() || bytes[i + 4].is_whitespace();
            if prev_ok && next_ok {
                // check if already followed by -S
                let rest: String = bytes[i + 4..].iter().collect();
                let trimmed = rest.trim_start();
                if trimmed.starts_with("-S") {
                    out.push_str("sudo");
                    i += 4;
                    continue;
                } else {
                    out.push_str("sudo -S -p ''");
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

async fn run_bash_with_password(command: &str, password: &str) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let injected = inject_sudo_s(command);
    let mut child = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(&injected)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn error: {}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        let pw = format!("{}\n", password);
        stdin
            .write_all(pw.as_bytes())
            .await
            .map_err(|e| format!("stdin error: {}", e))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("stdin flush: {}", e))?;
        drop(stdin);
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("wait error: {}", e))?;
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

pub async fn grep(pattern: &str, path: Option<&str>) -> Result<String, String> {
    let base = path.unwrap_or(".");
    let base_path = Path::new(base);
    if !base_path.exists() {
        return Err(format!("Path not found: {}", base));
    }
    let mut results = Vec::new();
    let walker = walkdir::WalkDir::new(base_path)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with(".git") && name != "target" && name != "node_modules"
        });
    for entry in walker.filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        results.push(format!(
                            "{}:{}: {}",
                            entry.path().display(),
                            idx + 1,
                            line.trim()
                        ));
                        if results.len() >= 200 {
                            break;
                        }
                    }
                }
            }
            if results.len() >= 200 {
                break;
            }
        }
    }
    if results.is_empty() {
        Ok(format!("No matches for '{}' in {}", pattern, base))
    } else {
        Ok(truncate_output(&results.join("\n"), TruncateStrategy::Head))
    }
}

pub async fn find(pattern: &str, path: Option<&str>) -> Result<String, String> {
    let base = path.unwrap_or(".");
    let base_path = Path::new(base);
    if !base_path.exists() {
        return Err(format!("Path not found: {}", base));
    }
    let mut results = Vec::new();
    let walker = walkdir::WalkDir::new(base_path)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with(".git") && name != "target"
        });
    let matcher = wildmatch::WildMatch::new(pattern);
    for entry in walker.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy();
        if matcher.matches(&name) || entry.path().to_string_lossy().contains(pattern) {
            results.push(entry.path().display().to_string());
            if results.len() >= 200 {
                break;
            }
        }
    }
    if results.is_empty() {
        Ok(format!("No files matching '{}' in {}", pattern, base))
    } else {
        Ok(truncate_output(&results.join("\n"), TruncateStrategy::Head))
    }
}

pub async fn ls(path: Option<&str>) -> Result<String, String> {
    let base = path.unwrap_or(".");
    let p = Path::new(base);
    if !p.exists() {
        return Err(format!("Path not found: {}", base));
    }
    if p.is_file() {
        return Ok(p.display().to_string());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(p).map_err(|e| format!("read dir error: {}", e))? {
        let e = entry.map_err(|e| format!("entry error: {}", e))?;
        let ft = e
            .file_type()
            .map_err(|e| format!("file type error: {}", e))?;
        let name = e.file_name().to_string_lossy().to_string();
        let suffix = if ft.is_dir() { "/" } else { "" };
        entries.push(format!("{}{}", name, suffix));
    }
    entries.sort();
    Ok(truncate_output(&entries.join("\n"), TruncateStrategy::Head))
}

async fn guard_path(path: &str, tool: &str) -> Option<String> {
    if crate::approval::is_auto_accept() {
        return None;
    }
    // returns Some(block_message) if denied, None if allowed
    let risk = crate::dir_guard::analyze_path(path)?;
    let reason_str = risk.reasons.join("; ");
    let display = format!("{} {}", tool, path);
    // Reuse approval system but tag as dir-guard via reason prefix
    let approved = {
        let fut = crate::approval::request(
            display.clone(),
            crate::bash_guard::Severity::High,
            risk.reasons.clone(),
        );
        match tokio::time::timeout(std::time::Duration::from_secs(300), fut).await {
            Ok(v) => v,
            Err(_) => false, // hard wall: deny on timeout
        }
    };
    if !approved {
        return Some(format!(
            "[dir-guard BLOCKED (HIGH): {}]\n{}: {}\nHint: outside CWD '{}' — approve with [a]/[A] or add to ~/.lean/dir_allowlist.json or run with LEAN_DIR_GUARD_DISABLED=1",
            reason_str, tool, path, crate::dir_guard::project_root().display()
        ));
    }
    None
}

async fn guard_bash(cmd: &str) -> Option<String> {
    if crate::approval::is_auto_accept() {
        return None;
    }
    // Check both guards and merge; dir-guard is HIGH severity hard wall
    // Returns Some(string) if the caller should return that string directly (blocked OR approved-medium annotated output).
    // Returns None if allowed to proceed to normal run_bash.
    let bash_risk = crate::bash_guard::analyze(cmd);
    let dir_risk = crate::dir_guard::analyze_bash(cmd);

    if bash_risk.is_none() && dir_risk.is_none() {
        return None;
    }

    let mut reasons = Vec::new();
    let mut severity = crate::bash_guard::Severity::Medium;
    let mut is_dir_guard = false;

    if let Some(r) = &bash_risk {
        reasons.extend(r.reasons.clone());
        if r.severity == crate::bash_guard::Severity::High {
            severity = crate::bash_guard::Severity::High;
        }
    }
    if let Some(r) = &dir_risk {
        reasons.extend(r.reasons.clone());
        severity = crate::bash_guard::Severity::High;
        is_dir_guard = true;
    }

    let reasons_clone = reasons.clone();
    let sev_clone = severity.clone();
    let approved = {
        let fut = crate::approval::request(cmd.to_string(), sev_clone, reasons_clone);
        match tokio::time::timeout(std::time::Duration::from_secs(300), fut).await {
            Ok(v) => v,
            Err(_) => {
                if is_dir_guard {
                    false
                } else {
                    match severity {
                        crate::bash_guard::Severity::High => false,
                        crate::bash_guard::Severity::Medium => true,
                    }
                }
            }
        }
    };
    if !approved {
        let sev_str = match severity {
            crate::bash_guard::Severity::High => "HIGH",
            crate::bash_guard::Severity::Medium => "MEDIUM",
        };
        let guard = if is_dir_guard {
            "dir-guard"
        } else {
            "bash-guard"
        };
        let reason_str = reasons.join("; ");
        return Some(format!("[{} BLOCKED ({}): {}]\nCommand: {}\nHint: {} — use allowlist or LEAN_DIR_GUARD_DISABLED=1 / --bash-guard-disabled", guard, sev_str, reason_str, cmd, if is_dir_guard { format!("outside CWD '{}'", crate::dir_guard::project_root().display()) } else { "risky command".to_string() }));
    }
    // Approved
    if !is_dir_guard && severity == crate::bash_guard::Severity::Medium {
        // Medium bash risk: run and annotate, return directly to avoid second prompt
        let out = run_bash(cmd)
            .await
            .unwrap_or_else(|e| format!("Error: {}", e));
        let reason_str = reasons.join("; ");
        return Some(format!(
            "[bash-guard approved (MEDIUM): {}]\n{}",
            reason_str, out
        ));
    }
    // High dir-guard or high bash-guard approved: proceed to normal execution (no annotation needed)
    // But we already approved, so just allow normal run_bash without re-prompting. To avoid re-prompt,
    // we temporarily allowlist this exact command for the next call? Instead we run here and return.
    if is_dir_guard || severity == crate::bash_guard::Severity::High {
        let out = run_bash(cmd)
            .await
            .unwrap_or_else(|e| format!("Error: {}", e));
        // Add a small prefix so user knows it was dir-guarded but approved
        if is_dir_guard {
            return Some(format!(
                "[dir-guard approved (HIGH): {}]\n{}",
                reasons.join("; "),
                out
            ));
        }
        return Some(out);
    }
    None
}

async fn guard_mcp(server: &str, tool: &str, args: &serde_json::Value) -> Option<String> {
    if crate::approval::is_auto_accept() {
        return None;
    }
    // Every MCP tool call requires approval (HIGH)
    let display = format!("{}__{} {}", server, tool, args);
    let reasons = vec![format!("MCP tool {}.{} requires approval", server, tool)];
    let approved = {
        let fut = crate::approval::request(
            display.clone(),
            crate::bash_guard::Severity::High,
            reasons.clone(),
        );
        match tokio::time::timeout(std::time::Duration::from_secs(300), fut).await {
            Ok(v) => v,
            Err(_) => false,
        }
    };
    if !approved {
        return Some(format!(
            "[mcp-guard BLOCKED (HIGH): MCP tool requires approval]\n{}__{} {}",
            server, tool, args
        ));
    }
    None
}

async fn run_subagent(agent_name: &str, task: &str, label: &str) -> String {
    if label.trim().is_empty() {
        return "Error: subagent 'name' is required — provide a unique label (e.g. 'research-auth')".to_string();
    }
    let agent = match crate::agents::load_agent(agent_name).await {
        Ok(a) => a,
        Err(e) => return format!("Error: {}", e),
    };
    // auto-suffix if duplicate label among running agents
    let existing: std::collections::HashSet<String> = crate::agents::list_subagents()
        .into_iter()
        .filter(|s| s.status == "running")
        .map(|s| s.label.clone())
        .collect();
    let mut display_label = label.trim().to_string();
    if existing.contains(&display_label) {
        let mut n = 2;
        loop {
            let cand = format!("{}-{}", label.trim(), n);
            if !existing.contains(&cand) {
                display_label = cand;
                break;
            }
            n += 1;
        }
    }
    let id = format!("{}-{}", display_label, &uuid_simple());
    crate::agents::register_subagent(
        id.clone(),
        agent_name.to_string(),
        display_label.clone(),
        task.to_string(),
    );
    let sub_prompt = format!(
        "Agent: {}\nDescription: {}\n\nTask: {}\n\nContext:\n{}",
        agent.name, agent.description, task, agent.body
    );
    let model = agent
        .model
        .clone()
        .unwrap_or_else(|| crate::llm::DEFAULT_MODEL.to_string());
    let max_steps = 15usize;
    let id_clone = id.clone();
    let display_clone = display_label.clone();
    let task_clone = task.to_string();
    let agent_name_clone = agent_name.to_string();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Handle::try_current();
        let handle = rt.ok();
        // Use a new current_thread runtime if no handle
        let fut = async move {
            use futures::StreamExt;
            let started = std::time::SystemTime::now();
            let mut stream = crate::agent::run_agent(sub_prompt, model, max_steps);
            futures::pin_mut!(stream);
            let mut final_text = String::new();
            let mut last_error: Option<String> = None;
            let mut tool_start_times: std::collections::HashMap<String, std::time::Instant> =
                std::collections::HashMap::new();
            crate::agents::append_subagent_msg(
                &id_clone,
                crate::agents::SubagentMsg {
                    role: "system".to_string(),
                    content: format!("[{}] started: {}", agent_name_clone, task_clone),
                    tool_name: None,
                    tool_args: None,
                    tool_id: None,
                    elapsed_ms: None,
                },
            );
            while let Some(ev) = stream.next().await {
                match ev {
                    crate::agent::AgentEvent::Text { delta } => {
                        final_text.push_str(&delta);
                        if !delta.trim().is_empty() {
                            crate::agents::append_subagent_msg(
                                &id_clone,
                                crate::agents::SubagentMsg {
                                    role: "assistant".to_string(),
                                    content: delta.clone(),
                                    tool_name: None,
                                    tool_args: None,
                                    tool_id: None,
                                    elapsed_ms: None,
                                },
                            );
                        }
                    }
                    crate::agent::AgentEvent::Reasoning { delta } => {
                        if !delta.trim().is_empty() {
                            crate::agents::append_subagent_msg(
                                &id_clone,
                                crate::agents::SubagentMsg {
                                    role: "thinking".to_string(),
                                    content: delta.clone(),
                                    tool_name: None,
                                    tool_args: None,
                                    tool_id: None,
                                    elapsed_ms: None,
                                },
                            );
                        }
                    }
                    crate::agent::AgentEvent::ToolStart {
                        name,
                        args,
                        id: tool_id,
                    } => {
                        let args_str =
                            serde_json::to_string(&args).unwrap_or_else(|_| format!("{:?}", args));
                        tool_start_times.insert(tool_id.clone(), std::time::Instant::now());
                        crate::agents::append_subagent_msg(
                            &id_clone,
                            crate::agents::SubagentMsg {
                                role: "tool".to_string(),
                                content: format!("{} {}", name, args_str),
                                tool_name: Some(name.clone()),
                                tool_args: Some(args_str),
                                tool_id: Some(tool_id),
                                elapsed_ms: None,
                            },
                        );
                    }
                    crate::agent::AgentEvent::ToolResult {
                        name,
                        result,
                        id: tool_id,
                        elapsed_ms,
                    } => {
                        if result.contains("Error") && name == "subagent" {
                            last_error = Some(result.clone());
                        }
                        let elapsed = if elapsed_ms > 0 {
                            elapsed_ms
                        } else if let Some(start) = tool_start_times.remove(&tool_id) {
                            start.elapsed().as_millis() as u64
                        } else {
                            0
                        };
                        crate::agents::append_subagent_msg(
                            &id_clone,
                            crate::agents::SubagentMsg {
                                role: "tool".to_string(),
                                content: format!("{} → {}", name, result),
                                tool_name: Some(name.clone()),
                                tool_args: None,
                                tool_id: Some(tool_id),
                                elapsed_ms: Some(elapsed),
                            },
                        );
                    }
                    crate::agent::AgentEvent::Done { text, .. } => {
                        final_text = text;
                        break;
                    }
                    _ => {}
                }
            }
            let result_text = if final_text.trim().is_empty() {
                if let Some(e) = last_error {
                    crate::agents::update_subagent(&id_clone, "error");
                    format!("[subagent {} error] {}", display_clone, e)
                } else {
                    crate::agents::append_subagent_transcript(
                        &id_clone,
                        "[error] no output".to_string(),
                    );
                    crate::agents::update_subagent(&id_clone, "error");
                    format!("[subagent {}] no output", display_clone)
                }
            } else {
                crate::agents::append_subagent_transcript(
                    &id_clone,
                    format!("done: {} chars", final_text.len()),
                );
                crate::agents::update_subagent(&id_clone, "done");
                format!("[subagent:{}]\n{}", display_clone, final_text)
            };
            crate::agents::append_subagent_msg(
                &id_clone,
                crate::agents::SubagentMsg {
                    role: "system".to_string(),
                    content: result_text.clone(),
                    tool_name: None,
                    tool_args: None,
                    tool_id: None,
                    elapsed_ms: None,
                },
            );
            let elapsed_ms = std::time::SystemTime::now()
                .duration_since(started)
                .unwrap_or_default()
                .as_millis() as u64;
            crate::agents::push_wake_tool(crate::agents::WakeMessage {
                id: id_clone.clone(),
                agent: agent_name_clone.clone(),
                label: display_clone.clone(),
                task: task_clone.clone(),
                result: result_text.clone(),
                elapsed_ms,
            });
            // keep done visible for 2s then remove from session
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            crate::agents::remove_subagent(&id_clone);
        };
        if let Some(h) = handle {
            let _ = h.block_on(fut);
        } else {
            let rt2 = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _ = rt2.block_on(fut);
        }
    });
    format!("[subagent:{} started — running in background, you can keep working; you will be woken when done]", display_label)
}

fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos & 0xffffff)
}

pub async fn execute_tool(name: &str, args: serde_json::Value) -> String {
    // MCP namespaced tools: server__tool
    if name.contains("__") {
        // Check if this is an MCP tool (if registry has it) or treat any __ as MCP
        // For now, route any __ to MCP; if server not found, mcp::call_tool will error
        if let Some((server, tool)) = name.split_once("__") {
            if let Some(blocked) = guard_mcp(server, tool, &args).await {
                return blocked;
            }
            return crate::mcp::call_tool(server, tool, args)
                .await
                .unwrap_or_else(|e| format!("Error: {}", e));
        }
    }
    let res = match name {
        "read" | "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(blocked) = guard_path(path, "read").await {
                return blocked;
            }
            read_file(path).await
        }
        "write" | "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(blocked) = guard_path(path, "write").await {
                return blocked;
            }
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            write_file(path, content).await
        }
        "edit" | "edit_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(blocked) = guard_path(path, "edit").await {
                return blocked;
            }
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
        "grep" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = args.get("path").and_then(|v| v.as_str());
            grep(pattern, path).await
        }
        "find" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = args.get("path").and_then(|v| v.as_str());
            find(pattern, path).await
        }
        "ls" => {
            let path = args.get("path").and_then(|v| v.as_str());
            ls(path).await
        }
        "web_fetch" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            web_fetch(url).await
        }
        "read_agent" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match crate::agents::load_agent(name).await {
                Ok(a) => Ok(a.content),
                Err(e) => Err(format!("{}", e)),
            }
        }
        "subagents_list" => {
            let subs = crate::agents::list_subagents();
            let running: Vec<_> = subs.into_iter().filter(|s| s.status == "running").collect();
            if running.is_empty() {
                Ok("No live background subagents — all done or none spawned.".to_string())
            } else {
                let mut lines = Vec::new();
                for s in running {
                    let elapsed = std::time::SystemTime::now()
                        .duration_since(s.started_at)
                        .unwrap_or_default();
                    let secs = elapsed.as_secs();
                    let task_preview = if s.task.len() > 80 {
                        format!("{}…", &s.task[..80])
                    } else {
                        s.task.clone()
                    };
                    let short_id = &s.id[..8.min(s.id.len())];
                    let label = if s.label.is_empty() {
                        s.agent.clone()
                    } else {
                        s.label.clone()
                    };
                    lines.push(format!(
                        "- {} {} [{}] {} — \"{}\" ({}s, {} transcript lines)",
                        label,
                        short_id,
                        s.status,
                        s.agent,
                        task_preview,
                        secs,
                        s.transcript.len()
                    ));
                }
                Ok(lines.join("\n"))
            }
        }
        "subagent" => {
            let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("");
            let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
            let label = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            Ok(run_subagent(agent, task, label).await)
        }
        "bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(result) = guard_bash(cmd).await {
                return result;
            }
            if contains_sudo(cmd) {
                let mut attempts = 0;
                loop {
                    attempts += 1;
                    let pw_opt = crate::sudo::request(cmd.to_string()).await;
                    match pw_opt {
                        None => {
                            return "[sudo cancelled by user — command not executed]".to_string()
                        }
                        Some(pw) => {
                            let res = run_bash_with_password(cmd, &pw).await;
                            drop(pw);
                            match res {
                                Ok(out)
                                    if out.contains("Sorry, try again")
                                        || out.contains("incorrect password") =>
                                {
                                    if attempts >= 3 {
                                        return format!(
                                            "{}\n[sudo: 3 failed attempts — giving up]",
                                            out
                                        );
                                    }
                                    continue;
                                }
                                Ok(out) => return out,
                                Err(e) if e.contains("Sorry, try again") && attempts < 3 => {
                                    continue
                                }
                                Err(e) => return format!("Error: {}", e),
                            }
                        }
                    }
                }
            }
            run_bash(cmd).await
        }
        "web_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            web_search(q).await
        }
        "ask_user" => {
            let questions = crate::question::parse_questions(&args);
            if questions.is_empty() {
                Err("ask_user requires at least one question with options".to_string())
            } else {
                Ok(crate::question::ask(questions).await)
            }
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
            let category = args
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("fact");
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let scope = args
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("global");
            Ok(crate::memory::api_remember(content, category, tags, scope))
        }
        "search_memory" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            Ok(crate::memory::api_search(q))
        }
        "recall_memory" => Ok(crate::memory::api_recall()),
        "list_memories" => {
            let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            Ok(crate::memory::api_list(tag))
        }
        "forget_memory" => {
            let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
            Ok(crate::memory::api_forget(id))
        }
        "consolidate_memory" => Ok(crate::memory::api_consolidate()),
        "memory_stats" => Ok(crate::memory::api_stats()),
        _ => Err(format!("unknown tool: {}", name)),
    };
    match res {
        Ok(s) => s,
        Err(e) => format!("Error: {}", e),
    }
}
