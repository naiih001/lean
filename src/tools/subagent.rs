pub async fn run_subagent(agent_name: &str, task: &str, label: &str) -> String {
    if label.trim().is_empty() {
        return "Error: subagent 'name' is required — provide a unique label (e.g. 'research-auth')".to_string();
    }
    let agent = match crate::services::agents::load_agent(agent_name).await {
        Ok(a) => a,
        Err(e) => return format!("Error: {}", e),
    };
    // auto-suffix if duplicate label among running agents
    let existing: std::collections::HashSet<String> = crate::services::agents::list_subagents()
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
    crate::services::agents::register_subagent(
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
        .unwrap_or_else(|| crate::integrations::llm::DEFAULT_MODEL.to_string());
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
            let stream = crate::agent::run_agent(sub_prompt, model, max_steps);
            futures::pin_mut!(stream);
            let mut final_text = String::new();
            let mut last_error: Option<String> = None;
            let mut tool_start_times: std::collections::HashMap<String, std::time::Instant> =
                std::collections::HashMap::new();
            crate::services::agents::append_subagent_msg(
                &id_clone,
                crate::services::agents::SubagentMsg {
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
                            crate::services::agents::append_subagent_msg(
                                &id_clone,
                                crate::services::agents::SubagentMsg {
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
                            crate::services::agents::append_subagent_msg(
                                &id_clone,
                                crate::services::agents::SubagentMsg {
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
                        crate::services::agents::append_subagent_msg(
                            &id_clone,
                            crate::services::agents::SubagentMsg {
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
                        crate::services::agents::append_subagent_msg(
                            &id_clone,
                            crate::services::agents::SubagentMsg {
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
                    crate::services::agents::update_subagent(&id_clone, "error");
                    format!("[subagent {} error] {}", display_clone, e)
                } else {
                    crate::services::agents::append_subagent_transcript(
                        &id_clone,
                        "[error] no output".to_string(),
                    );
                    crate::services::agents::update_subagent(&id_clone, "error");
                    format!("[subagent {}] no output", display_clone)
                }
            } else {
                crate::services::agents::append_subagent_transcript(
                    &id_clone,
                    format!("done: {} chars", final_text.len()),
                );
                crate::services::agents::update_subagent(&id_clone, "done");
                format!("[subagent:{}]\n{}", display_clone, final_text)
            };
            crate::services::agents::append_subagent_msg(
                &id_clone,
                crate::services::agents::SubagentMsg {
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
            crate::services::agents::push_wake_tool(crate::services::agents::WakeMessage {
                id: id_clone.clone(),
                agent: agent_name_clone.clone(),
                label: display_clone.clone(),
                task: task_clone.clone(),
                result: result_text.clone(),
                elapsed_ms,
            });
            // keep done visible for 2s then remove from session
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            crate::services::agents::remove_subagent(&id_clone);
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

pub(crate) fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos & 0xffffff)
}
