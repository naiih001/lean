use crate::tools::fs::{truncate_output, TruncateStrategy};

pub(crate) fn contains_sudo(cmd: &str) -> bool {
    // token-aware check for sudo as command
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    tokens.iter().any(|t| *t == "sudo") || cmd.contains("sudo ")
}

pub(crate) fn inject_sudo_s(cmd: &str) -> String {
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

pub(crate) async fn run_bash_with_password(
    command: &str,
    password: &str,
) -> Result<String, String> {
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
