pub mod bash;
pub mod fs;
pub mod search;
pub mod subagent;
pub mod web;

pub use fs::{edit_file, human_size, image_mime_type, is_image_file, read_file, truncate_output, write_file, TruncateStrategy};

use serde_json::Value;

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
        let out = bash::run_bash(cmd)
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
        let out = bash::run_bash(cmd)
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
            fs::read_file(path).await
        }
        "write" | "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(blocked) = guard_path(path, "write").await {
                return blocked;
            }
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            fs::write_file(path, content).await
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
            fs::edit_file(path, old2, new2).await
        }
        "grep" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = args.get("path").and_then(|v| v.as_str());
            search::grep(pattern, path).await
        }
        "find" => {
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = args.get("path").and_then(|v| v.as_str());
            search::find(pattern, path).await
        }
        "ls" => {
            let path = args.get("path").and_then(|v| v.as_str());
            search::ls(path).await
        }
        "web_fetch" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            web::web_fetch(url).await
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
            Ok(subagent::run_subagent(agent, task, label).await)
        }
        "bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(result) = guard_bash(cmd).await {
                return result;
            }
            if bash::contains_sudo(cmd) {
                let mut attempts = 0;
                loop {
                    attempts += 1;
                    let pw_opt = crate::sudo::request(cmd.to_string()).await;
                    match pw_opt {
                        None => {
                            return "[sudo cancelled by user — command not executed]".to_string()
                        }
                        Some(pw) => {
                            let res = bash::run_bash_with_password(cmd, &pw).await;
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
            bash::run_bash(cmd).await
        }
        "web_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            web::web_search(q).await
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
