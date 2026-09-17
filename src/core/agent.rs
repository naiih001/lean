use crate::integrations::llm::{self, Client};
use crate::services::skills;
use futures::Stream;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

// Re-exports from core modules for thin agent loop
pub(crate) use crate::core::history::{
    build_user_content, drop_orphaned_tool_outputs, history_slice_for_api, prune_context_messages,
    strip_images_for_non_vision,
};
pub(crate) use crate::core::modes::{is_ask_mode, is_plan_mode};
pub(crate) use crate::core::prompts::{
    build_system_prompt, truncate_chars, truncate_for_llm, truncate_str, truncate_to_bytes,
    ASK_READONLY_DENY_MSG, SYSTEM_PROMPT,
};
pub(crate) use crate::core::tracker::{
    is_mcp_read, is_mutating_tool, is_permission_to_leave_plan, is_plan_exempt_write,
    is_readonly_bash, is_stay_in_plan, PlanTracker, MAX_NOCALL_STREAK,
};

pub enum AgentEvent {
    Text {
        delta: String,
    },
    Reasoning {
        delta: String,
    },
    TextDone {
        text: String,
    },
    ToolStart {
        name: String,
        args: Value,
        id: String,
    },
    ToolResult {
        name: String,
        result: String,
        id: String,
        elapsed_ms: u64,
    },
    Step {
        n: usize,
    },
    Done {
        text: String,
        history: Vec<Value>,
    },
}

struct ToolAccum {
    id: String,
    name: String,
    args: String,
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500..=599)
}

fn is_retryable_error(e: &reqwest::Error) -> bool {
    e.is_timeout()
        || e.is_connect()
        || e.is_request() && format!("{:?}", e).to_lowercase().contains("connection")
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(v) = headers.get(reqwest::header::RETRY_AFTER) {
        if let Ok(s) = v.to_str() {
            // Try seconds as integer
            if let Ok(secs) = s.trim().parse::<u64>() {
                return Some(Duration::from_secs(secs));
            }
            // Try HTTP date (fallback: ignore, use backoff)
        }
    }
    None
}

async fn post_with_retry(
    client: &Client,
    url: String,
    body: &Value,
) -> Result<reqwest::Response, String> {
    // llms-sdk style: unified RetryPolicy (mirrors llms-sdk behaviour, keep lean's thin client)
    let policy = crate::integrations::llm::RetryPolicy::for_provider(&client.provider);
    let max_retries = policy.max_retries;
    let base_delays_ms = policy.base_delays_ms;
    let is_ollama = client.provider == crate::integrations::models::Provider::Ollama;
    let mut last_err: Option<String> = None;
    for attempt in 0..=max_retries {
        let res = client
            .apply_auth(client.http.post(url.clone()))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await;
        match res {
            Ok(resp) => {
                if resp.status().is_success() {
                    return Ok(resp);
                }
                let status = resp.status().as_u16();
                if is_retryable_status(status) && attempt < max_retries {
                    let retry_after = parse_retry_after(resp.headers())
                        .unwrap_or(Duration::from_millis(base_delays_ms[attempt as usize]));
                    let delay = std::cmp::min(retry_after, Duration::from_secs(30));
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    let txt = resp.text().await.unwrap_or_default();
                    let hint = if is_ollama {
                        " — Ollama is not running or model not pulled. Run 'ollama serve' and 'ollama pull <model>' and check http://localhost:11434/api/tags"
                    } else if status == 401 || status == 403 {
                        " — check API key (OPENAI_API_KEY / ANTHROPIC_API_KEY / OPENCODE_API_KEY)"
                    } else if status == 429 {
                        " — rate limited, try again shortly"
                    } else {
                        ""
                    };
                    // Truncate long body for inline display
                    let snippet = if txt.chars().count() > 800 {
                        format!("{}… [truncated]", txt.chars().take(800).collect::<String>())
                    } else {
                        txt
                    };
                    return Err(format!(
                        "[LLM HTTP {}: {}{}] (provider: {}, url: {})",
                        status,
                        snippet,
                        hint,
                        client.provider.as_str(),
                        url
                    ));
                }
            }
            Err(e) if is_retryable_error(&e) && attempt < max_retries => {
                let delay = Duration::from_millis(base_delays_ms[attempt as usize]);
                tokio::time::sleep(delay).await;
                last_err = Some(e.to_string());
                continue;
            }
            Err(e) => {
                let hint = if is_ollama
                    && (e.is_connect() || format!("{:?}", e).to_lowercase().contains("connection"))
                {
                    " — Ollama is not running — run 'ollama serve' and ensure http://localhost:11434/api/tags is reachable"
                } else if e.is_timeout() {
                    " — timeout"
                } else {
                    ""
                };
                return Err(format!(
                    "[LLM error: {}{}] (provider: {})",
                    e,
                    hint,
                    client.provider.as_str()
                ));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "[LLM error: max retries exceeded]".to_string()))
}

pub fn run_agent(
    user_prompt: String,
    model: String,
    max_steps: usize,
) -> impl Stream<Item = AgentEvent> {
    run_agent_with_history(user_prompt, model, max_steps, Vec::new())
}

pub fn run_agent_with_history(
    user_prompt: String,
    model: String,
    max_steps: usize,
    history: Vec<Value>,
) -> impl Stream<Item = AgentEvent> {
    async_stream::stream! {
        let resolved = match crate::integrations::models::resolve(Some(&model)) {
            Ok(r) => r,
            Err(e) => {
                yield AgentEvent::Text { delta: format!("\n[model resolve error: {}]", e) };
                yield AgentEvent::Done { text: String::new(), history: Vec::new() };
                return;
            }
        };
        let api_mode = resolved.api_mode.clone();
        let supports_vision = resolved.vision;
        if api_mode == crate::integrations::models::ApiMode::Responses {
            // Responses branch keeps a chat-shaped history for persistence, translating to input each turn
            let model_id = resolved.model.clone();
            let client = Client::from_resolved(&resolved);
            let system = build_system_prompt().await;
            let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system})];
            let hist_slice = history_slice_for_api(&history);
            for v in hist_slice {
                let mut val = v.clone();
                if let Some(content) = val.get("content") { if let Some(s) = content.as_str() { if s.chars().count() > 3000 { val["content"] = json!(format!("{}… [truncated]", truncate_chars(s, 3000))); } } }
                messages.push(val);
            }
            messages.push(json!({"role": "user", "content": build_user_content(&user_prompt)}));
            let mut final_text = String::new();
            let mut tracker = PlanTracker::new(&user_prompt);
            for step in 0..max_steps {
                yield AgentEvent::Step { n: step + 1 };
                let focus = tracker.focus_context(step + 1);
                // Re-inject focus each turn from a history with stale injected context removed,
                // so instructions stay bounded instead of accumulating across steps.
                let pruned = prune_context_messages(&messages);
                let pruned = if supports_vision { pruned } else {
                    if step == 0 && pruned.iter().any(|m| {
                        m.get("content").and_then(|c| c.as_array()).map(|arr|
                            arr.iter().any(|p| p.get("type").and_then(|t| t.as_str()) == Some("image_url"))
                        ).unwrap_or(false)
                    }) {
                        yield AgentEvent::Text { delta: "\n[image(s) omitted — model does not support vision]\n".to_string() };
                    }
                    strip_images_for_non_vision(&pruned)
                };
                // Responses is strict: every function_call_output must have a matching function_call.
                // history_slice_for_api can cut off the assistant turn but keep the tool output,
                // so drop orphans before converting to input (Chat is tolerant; Responses 400s).
                let mut pruned_for_input = pruned;
                drop_orphaned_tool_outputs(&mut pruned_for_input);
                let (mut instructions, mut input) = llm::chat_messages_to_responses_input(&pruned_for_input, &system);
                if input.is_empty() {
                    input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text": user_prompt.clone()}]}));
                }
                instructions = format!("{}\n\n{}", instructions, focus);
                // Build tools for responses
                let tools = llm::responses_tool_definitions().await;
                let body = llm::build_responses_request_body(&model_id, &instructions, &input, &tools);
                let resp = match post_with_retry(&client, client.responses_url(), &body).await {
                    Ok(r) => r,
                    Err(e) => { yield AgentEvent::Text { delta: format!("\n{}", e) }; break; }
                };
                let mut accum_text = String::new();
                let mut accum_reasoning = String::new();
                let mut tool_acc: HashMap<String, ToolAccum> = HashMap::new();
                let mut usage: Option<llm::Usage> = None;
                // For streaming, we need to handle ResponsesEvent SSE
                let mut stream = resp.bytes_stream();
                use futures::StreamExt;
                let mut buf = String::new();
                // For mapping output_index/call_id to tool entry when delta doesn't carry name
                let mut pending_calls: HashMap<String, String> = HashMap::new(); // item_id/call_id -> name placeholder
                let mut pending_deltas: HashMap<String, String> = HashMap::new(); // buffer for deltas that arrived before OutputItemAdded
                while let Some(chunk) = stream.next().await {
                    let bytes = match chunk { Ok(b) => b, Err(e) => { yield AgentEvent::Text { delta: format!("\n[stream error: {}]", e) }; break; } };
                    let text = String::from_utf8_lossy(&bytes);
                    buf.push_str(&text);
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].trim().to_string();
                        buf.drain(..pos+1);
                        if line.is_empty() { continue; }
                        // handle possible "event:" line
                        if line.starts_with("event:") { continue; }
                        let data = if let Some(d) = line.strip_prefix("data: ") { d.trim() } else { line.trim() };
                        if data == "[DONE]" { break; }
                        if data.is_empty() { continue; }
                        // Try responses event first
                        // debug for tool delta issues
                        if data.contains("function_call") || data.contains("delta") {
                            // eprintln!("[DEBUG] data: {}", data);
                        }
                        if let Some(ev) = llm::parse_responses_event(data) {
                            match ev {
                                llm::ResponsesEvent::OutputTextDelta { delta, .. } => {
                                    accum_text.push_str(&delta);
                                    yield AgentEvent::Text { delta };
                                },
                                llm::ResponsesEvent::OutputTextDone { text } => {
                                    if !accum_text.contains(&text) && !text.is_empty() {
                                        // ensure full text accounted if deltas missed
                                    }
                                },
                                llm::ResponsesEvent::ReasoningDelta { delta } | llm::ResponsesEvent::ReasoningTextDelta { delta } => {
                                    accum_reasoning.push_str(&delta);
                                    yield AgentEvent::Reasoning { delta };
                                },
                                llm::ResponsesEvent::OutputItemAdded { item } => {
                                    if item.item_type == "function_call" {
                                        let fc_id = item.id.clone().unwrap_or_default();
                                        let call_id = item.call_id.clone().unwrap_or_else(|| fc_id.clone());
                                        let name = item.name.clone().unwrap_or_default();
                                        if !call_id.is_empty() {
                                            let entry = tool_acc.entry(call_id.clone()).or_insert_with(|| ToolAccum { id: call_id.clone(), name: String::new(), args: String::new() });
                                            if !name.is_empty() { entry.name = name.clone(); }
                                            entry.id = call_id.clone();
                                            if !fc_id.is_empty() {
                                                pending_calls.insert(fc_id.clone(), call_id.clone());
                                            }
                                            // Flush any deltas that arrived before the item was announced
                                            for key in [fc_id.clone(), call_id.clone()] {
                                                if let Some(buf) = pending_deltas.remove(&key) {
                                                    entry.args.push_str(&buf);
                                                }
                                            }
                                            // Also check pending keyed by raw fc
                                            if let Some(buf) = pending_deltas.remove(&fc_id) {
                                                if !entry.args.contains(&buf) { entry.args.push_str(&buf); }
                                            }
                                        }
                                    }
                                },
                                llm::ResponsesEvent::OutputItemDone { item } => {
                                    if item.item_type == "function_call" {
                                        let fc_id = item.id.clone().unwrap_or_default();
                                        let call_id = item.call_id.clone().unwrap_or_else(|| fc_id.clone());
                                        let name = item.name.clone().unwrap_or_default();
                                        let key = if !call_id.is_empty() { call_id.clone() } else { fc_id.clone() };
                                        let resolved_key = if tool_acc.contains_key(&key) { key.clone() } else if let Some(mapped) = pending_calls.get(&fc_id) { mapped.clone() } else { key.clone() };
                                        if !resolved_key.is_empty() {
                                            let entry = tool_acc.entry(resolved_key.clone()).or_insert_with(|| ToolAccum { id: call_id.clone(), name: String::new(), args: String::new() });
                                            if !name.is_empty() { entry.name = name; }
                                            if let Some(args) = item.arguments { if !args.is_empty() && entry.args.is_empty() { entry.args = args; } }
                                            if !call_id.is_empty() { entry.id = call_id.clone(); }
                                            // Flush pending deltas now that we have the real id
                                            for k in [fc_id.clone(), call_id.clone(), resolved_key.clone()] {
                                                if let Some(buf) = pending_deltas.remove(&k) {
                                                    if entry.args.is_empty() { entry.args = buf; } else if !entry.args.contains(&buf) { entry.args.push_str(&buf); }
                                                }
                                            }
                                        }
                                    }
                                },
                                llm::ResponsesEvent::FunctionCallArgsDelta { delta, item_id, call_id, .. } => {
                                    let raw = call_id.clone().or(item_id.clone()).unwrap_or_default();
                                    let resolved = if let Some(mapped) = pending_calls.get(&raw) { mapped.clone() } else { raw.clone() };
                                    let key = if tool_acc.contains_key(&resolved) { resolved.clone() } else if tool_acc.contains_key(&raw) { raw.clone() } else { resolved.clone() };
                                    if key.is_empty() || !tool_acc.contains_key(&key) && !tool_acc.contains_key(&raw) && pending_calls.get(&raw).is_none() {
                                        // No tool entry yet — buffer until OutputItemAdded arrives (strict: never synthesize call_0)
                                        pending_deltas.entry(raw.clone()).or_default().push_str(&delta);
                                    } else {
                                        let target = if tool_acc.contains_key(&resolved) { resolved.clone() } else { raw.clone() };
                                        let entry = tool_acc.entry(target.clone()).or_insert_with(|| ToolAccum { id: target.clone(), name: String::new(), args: String::new() });
                                        entry.args.push_str(&delta);
                                    }
                                },
                                llm::ResponsesEvent::FunctionCallArgsDone { arguments, item_id, call_id, .. } => {
                                    // Try to map to correct tool via item_id/call_id
                                    let raw = call_id.clone().or(item_id.clone()).unwrap_or_default();
                                    let resolved = if let Some(mapped) = pending_calls.get(&raw) { mapped.clone() } else { raw.clone() };
                                    let target = if tool_acc.contains_key(&resolved) { Some(resolved.clone()) } else if tool_acc.contains_key(&raw) { Some(raw.clone()) } else { None };
                                    if let Some(k) = target {
                                        if let Some(entry) = tool_acc.get_mut(&k) {
                                            if entry.args.is_empty() {
                                                entry.args = arguments.clone();
                                            } else if entry.args != arguments {
                                                // delta already filled, done may be same — only update if different and empty or placeholder
                                                // avoid overwriting correctly streamed args
                                            }
                                        }
                                    } else {
                                        // No matching entry — try empty slot, else ignore (avoid duplicate call_0)
                                        let mut assigned = false;
                                        for (_, entry) in tool_acc.iter_mut() {
                                            if entry.args.is_empty() {
                                                entry.args = arguments.clone();
                                                assigned = true;
                                                break;
                                            }
                                        }
                                        if !assigned {
                                            // Check if any entry already has same arguments — likely duplicate done event, ignore
                                            let already_has = tool_acc.values().any(|e| e.args == arguments);
                                            if !already_has {
                                                // Only create new if truly unknown and no empty slot, but avoid call_0 duplicate
                                                // For safety, don't create; the tool call was already handled via delta
                                            }
                                        }
                                    }
                                },
                                llm::ResponsesEvent::Completed { response } => {
                                    if let Some(u) = response.usage { usage = Some(u); }
                                },
                                llm::ResponsesEvent::Unknown => {
                                    // try fallback to chat chunk if model incorrectly returned chat format on responses endpoint
                                    if let Some(chunk) = llm::maybe_chat_chunk(data) {
                                        for ch in chunk.choices {
                                            if let Some(t) = ch.delta.content { accum_text.push_str(&t); yield AgentEvent::Text { delta: t }; }
                                            if let Some(r) = ch.delta.reasoning_content.or(ch.delta.reasoning) { accum_reasoning.push_str(&r); yield AgentEvent::Reasoning { delta: r }; }
                                            if let Some(tcs) = ch.delta.tool_calls {
                                                for tc in tcs {
                                                    let key = tc.id.clone().unwrap_or_else(|| format!("idx_{}", tc.index));
                                                    let entry = tool_acc.entry(key.clone()).or_insert_with(|| ToolAccum { id: tc.id.clone().unwrap_or(key.clone()), name: String::new(), args: String::new() });
                                                    if let Some(id) = tc.id { if !id.is_empty() { entry.id = id; } }
                                                    if let Some(f) = tc.function { if let Some(n) = f.name { if !n.is_empty() { entry.name = n; } } if let Some(a) = f.arguments { entry.args.push_str(&a); } }
                                                }
                                            }
                                        }
                                        if let Some(u) = chunk.usage { usage = Some(u); }
                                    }
                                },
                            }
                        } else {
                            // Not a responses event, try chat chunk fallback
                            if let Some(chunk) = llm::maybe_chat_chunk(data) {
                                if let Some(u) = chunk.usage { usage = Some(u); }
                                for ch in chunk.choices {
                                    if let Some(t) = ch.delta.content { accum_text.push_str(&t); yield AgentEvent::Text { delta: t }; }
                                    if let Some(r) = ch.delta.reasoning_content.or(ch.delta.reasoning) { accum_reasoning.push_str(&r); yield AgentEvent::Reasoning { delta: r }; }
                                    if let Some(tcs) = ch.delta.tool_calls { for tc in tcs { let key = tc.id.clone().unwrap_or_else(|| format!("idx_{}", tc.index)); let entry = tool_acc.entry(key.clone()).or_insert_with(|| ToolAccum { id: tc.id.clone().unwrap_or(key), name: String::new(), args: String::new() }); if let Some(id)=tc.id { if !id.is_empty(){entry.id=id;} } if let Some(f)=tc.function { if let Some(n)=f.name { if !n.is_empty(){entry.name=n;}} if let Some(a)=f.arguments {entry.args.push_str(&a);} } } }
                                }
                            }
                        }
                    }
                }
                for line in buf.lines() {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    if line.starts_with("event:") { continue; }
                    let data = if let Some(d) = line.strip_prefix("data: ") { d.trim() } else { line };
                    if data == "[DONE]" { continue; }
                    if let Some(ev) = llm::parse_responses_event(data) {
                        match ev {
                            llm::ResponsesEvent::OutputTextDelta { delta, .. } => { accum_text.push_str(&delta); yield AgentEvent::Text { delta }; },
                            llm::ResponsesEvent::ReasoningDelta { delta } | llm::ResponsesEvent::ReasoningTextDelta { delta } => { accum_reasoning.push_str(&delta); yield AgentEvent::Reasoning { delta }; },
                            llm::ResponsesEvent::OutputItemAdded { item } => {
                                if item.item_type == "function_call" {
                                    let fc_id = item.id.clone().unwrap_or_default();
                                    let call_id = item.call_id.clone().unwrap_or_else(|| fc_id.clone());
                                    let name = item.name.clone().unwrap_or_default();
                                    if !call_id.is_empty() {
                                        let e = tool_acc.entry(call_id.clone()).or_insert_with(|| ToolAccum { id: call_id.clone(), name: String::new(), args: String::new() });
                                        if !name.is_empty() { e.name = name; }
                                        e.id = call_id.clone();
                                        if !fc_id.is_empty() { pending_calls.insert(fc_id.clone(), call_id.clone()); }
                                        for k in [fc_id.clone(), call_id.clone()] { if let Some(buf) = pending_deltas.remove(&k) { e.args.push_str(&buf); } }
                                    }
                                }
                            },
                            llm::ResponsesEvent::FunctionCallArgsDelta { delta, item_id, call_id, .. } => {
                                let raw = call_id.clone().or(item_id.clone()).unwrap_or_default();
                                let resolved = if let Some(mapped) = pending_calls.get(&raw) { mapped.clone() } else { raw.clone() };
                                if tool_acc.contains_key(&resolved) || tool_acc.contains_key(&raw) {
                                    let target = if tool_acc.contains_key(&resolved) { resolved } else { raw };
                                    let e = tool_acc.entry(target.clone()).or_insert_with(|| ToolAccum{id: target.clone(), name:String::new(), args:String::new()});
                                    e.args.push_str(&delta);
                                } else {
                                    pending_deltas.entry(raw.clone()).or_default().push_str(&delta);
                                }
                            },
                            _ => {}
                        }
                    } else if let Ok(chunk) = serde_json::from_str::<llm::ChatChunk>(data) {
                        for ch in chunk.choices { if let Some(t)=ch.delta.content{accum_text.push_str(&t); yield AgentEvent::Text{delta:t}; } if let Some(tcs)=ch.delta.tool_calls { for tc in tcs { let key=tc.id.clone().unwrap_or_else(||format!("idx_{}",tc.index)); let e=tool_acc.entry(key.clone()).or_insert_with(||ToolAccum{id:tc.id.clone().unwrap_or(key),name:String::new(),args:String::new()}); if let Some(id)=tc.id{if !id.is_empty(){e.id=id;}} if let Some(f)=tc.function{if let Some(n)=f.name{if !n.is_empty(){e.name=n;}} if let Some(a)=f.arguments{e.args.push_str(&a);}} } } }
                    }
                }
                // Flush any buffered deltas that arrived before OutputItemAdded (strict: no call_0 synthesis)
                if !pending_deltas.is_empty() {
                    let buffered: Vec<(String,String)> = pending_deltas.drain().collect();
                    for (raw, buf) in buffered {
                        let resolved = pending_calls.get(&raw).cloned().unwrap_or(raw.clone());
                        if let Some(e) = tool_acc.get_mut(&resolved) { e.args.push_str(&buf); }
                        else if let Some(e) = tool_acc.get_mut(&raw) { e.args.push_str(&buf); }
                        else if tool_acc.len() == 1 { if let Some((_, e)) = tool_acc.iter_mut().next() { e.args.push_str(&buf); } }
                    }
                }
                if !accum_text.is_empty() { final_text.push_str(&accum_text); yield AgentEvent::TextDone { text: accum_text.clone() }; tracker.record_text(&accum_text); }
                // If no tool calls, handle conversational / continuation logic same as chat
                if tool_acc.is_empty() {
                    if tracker.is_conversational_goal() {
                        if !accum_text.is_empty() { {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                } }
                        yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                        break;
                    }
                    tracker.nocall_streak += 1;
                    let complete = tracker.looks_complete(&accum_text);
                    // State-based completion: phrase alone is not enough for issue-solving; require evidence gate.
                    // Also gate the nocall-streak fallback with can_complete to prevent premature summary.
                    if complete || (tracker.nocall_streak >= MAX_NOCALL_STREAK && tracker.can_complete()) {
                        if !accum_text.is_empty() { {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                } }
                        yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                        break;
                    }
                    if !accum_text.is_empty() { {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                } }
                    continue;
                }
                // Tool calls present — sort by id for deterministic order
                let mut ordered: Vec<(String, ToolAccum)> = tool_acc.into_iter().collect();
                ordered.sort_by(|a,b| a.0.cmp(&b.0));
                let tool_names: Vec<String> = ordered.iter().map(|(_, acc)| acc.name.clone()).collect();
                tracker.record_tools(&tool_names);
                let mut tool_results: Vec<(String, String, String, Value)> = Vec::new();
                for (_, acc) in &ordered {
                    let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                    yield AgentEvent::ToolStart { name: acc.name.clone(), args: args_val.clone(), id: acc.id.clone() };
                }
                let gate_active = tracker.requires_approval() && !tracker.has_approval();
                let is_ask = is_ask_mode();
                let futs: Vec<_> = ordered.iter().map(|(_, acc)| { let name=acc.name.clone(); let id=acc.id.clone(); let args_val: Value=serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone())); let is_bash_readonly = name == "bash" && is_readonly_bash(args_val.get("command").and_then(|v| v.as_str()).unwrap_or("")); let is_mcp_readonly = name.contains("__") && is_mcp_read(&name); let ask_blocked = is_ask && is_mutating_tool(&name) && !is_bash_readonly && !is_mcp_readonly; let plan_blocked = gate_active && is_mutating_tool(&name) && !is_plan_exempt_write(&name, &args_val) && !is_bash_readonly && !is_mcp_readonly && !crate::guards::approval::is_auto_accept(); async move { let start=std::time::Instant::now(); let result = if ask_blocked { format!("[ASK BLOCKED] '{}' is blocked — {}. Allowed: read, web_search, grep, find, ls, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, 2>/dev/null), MCP reads. (Shift+Tab to cycle NORM/PLAN/ASK/AUTO)", name, ASK_READONLY_DENY_MSG) } else if plan_blocked { format!("[GATING BLOCKED — plan mode] Mutating tool '{}' is blocked until you complete Phases 1-4 and get explicit user approval via ask_user with '\\u{{2713}} Proceed as proposed'. Call ask_user now to clarify scope/approach. In plan mode only .lean/plans writes + read-only bash (2>/dev/null, pipes) + MCP reads are allowed before approval; all other mutations blocked. (Shift+Tab to cycle NORM/PLAN/ASK/AUTO or /plan to toggle.)", name) } else { crate::tools::execute_tool(&name, args_val.clone()).await }; let elapsed_ms=start.elapsed().as_millis() as u64; (id,name,result,args_val,elapsed_ms) }}).collect();
                let results = futures::future::join_all(futs).await;
                for (id, name, result, args_val, elapsed_ms) in results {
                    let display = result.find("<<IMAGE:").map_or_else(|| result.clone(), |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()));
                    yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                    // Classify tool outcome for evidence-driven tracker
                    tracker.note_tool_result(&name, &args_val, &result);
                    // If this was ask_user, check for Proceed approval
                    if name == "ask_user" {
                        tracker.note_ask_result(&result);
                    }
                    tool_results.push((id,name,result,args_val));
                }
                let tool_calls_json: Vec<Value> = ordered.iter().map(|(_, acc)| json!({"id": acc.id, "type": "function", "function": {"name": acc.name, "arguments": acc.args}})).collect();
                {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone(), "tool_calls": tool_calls_json.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                }
                for (id, _name, result, _) in tool_results {
                    // Option A: keep tool content string-only, send image as follow-up user message
                    if let Some(img_start) = result.find("<<IMAGE:") {
                        let meta = result[..img_start].trim_end();
                        let after = &result[img_start+8..];
                        if let Some(colon_pos) = after.find(':') {
                            let mime = &after[..colon_pos];
                            let b64_raw = after[colon_pos+1..].trim_end_matches(">>");
                            const MAX_B64_LEN: usize = 68_000; // ~50KB
                            let tool_text = if meta.is_empty() { "Image read follows in next message." } else { meta };
                            let truncated_tool = truncate_for_llm(tool_text);
                            messages.push(json!({"role": "tool", "tool_call_id": id, "content": truncated_tool}));
                            if b64_raw.len() > MAX_B64_LEN {
                                messages.push(json!({"role": "user", "content": format!("[image from tool {} omitted — {:.1} KB too large, max 50KB]", _name, b64_raw.len() as f64 * 0.75 / 1024.0)}));
                            } else {
                                let is_valid = b64_raw.chars().all(|c| c.is_ascii_alphanumeric() || c=='+' || c=='/' || c=='=');
                                if !is_valid {
                                    messages.push(json!({"role": "user", "content": "[image data invalid — omitted]"}));
                                } else {
                                    messages.push(json!({"role": "user", "content": [{"type":"text","text": format!("[image from {}: {}]", _name, mime)},{"type":"image_url","image_url":{"url": format!("data:{};base64,{}", mime, b64_raw)}}]}));
                                }
                            }
                            continue;
                        }
                    }
                    let truncated = truncate_for_llm(&result);
                    messages.push(json!({"role": "tool", "tool_call_id": id, "content": truncated}));
                }
                drop_orphaned_tool_outputs(&mut messages);
                let _ = usage;
            }
            yield AgentEvent::Done { text: final_text, history: messages.clone() };
            return;
        }
        // ── Chat Completions branch (original) ────────────────────────────────
        let model_id = resolved.model.clone();
        let client = Client::from_resolved(&resolved);
        let system = build_system_prompt().await;
        let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system})];
        let hist_slice = history_slice_for_api(&history);
        for v in hist_slice {
            let mut val = v.clone();
            if let Some(content) = val.get("content") { if let Some(s) = content.as_str() { if s.chars().count() > 3000 { val["content"] = json!(format!("{}… [truncated]", truncate_chars(s, 3000))); } } }
            messages.push(val);
        }
        messages.push(json!({"role": "user", "content": build_user_content(&user_prompt)}));
        let mut final_text = String::new();
        let mut tracker = PlanTracker::new(&user_prompt);
            for step in 0..max_steps {
                yield AgentEvent::Step { n: step + 1 };
                // Drop any stale focus/continue context, then re-inject a fresh focus
            // message right after the system prompt so it never accumulates.
            messages = prune_context_messages(&messages);
            let focus_msg = json!({"role": "system", "content": tracker.focus_context(step + 1)});
            if messages.len() > 1 { messages.insert(1, focus_msg); } else { messages.push(focus_msg); }
            let tools = llm::tool_definitions().await;
            let send_messages = if supports_vision { messages.clone() } else {
                // Warn once on first step if images are present
                if step == 0 && messages.iter().any(|m| {
                    m.get("content").and_then(|c| c.as_array()).map(|arr|
                        arr.iter().any(|p| p.get("type").and_then(|t| t.as_str()) == Some("image_url"))
                    ).unwrap_or(false)
                }) {
                    yield AgentEvent::Text { delta: "\n[image(s) omitted — model does not support vision]\n".to_string() };
                }
                strip_images_for_non_vision(&messages)
            };
            let body = json!({"model": model_id, "messages": send_messages, "tools": tools, "tool_choice": "auto", "stream": true});
            let resp = match post_with_retry(&client, client.chat_url(), &body).await {
                Ok(r) => r,
                Err(e) => { yield AgentEvent::Text { delta: format!("\n{}", e) }; break; }
            };
            let mut accum_text = String::new();
            let mut accum_reasoning = String::new();
            let mut tool_acc: HashMap<usize, ToolAccum> = HashMap::new();
            let mut usage: Option<llm::Usage> = None;
            let mut stream = resp.bytes_stream();
            use futures::StreamExt;
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                let bytes = match chunk { Ok(b) => b, Err(e) => { yield AgentEvent::Text { delta: format!("\n[stream error: {}]", e) }; break; } };
                let text = String::from_utf8_lossy(&bytes);
                buf.push_str(&text);
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf.drain(..pos+1);
                    if line.is_empty() { continue; }
                    let data = if let Some(d) = line.strip_prefix("data: ") { d } else { continue };
                    if data == "[DONE]" { break; }
                    let parsed: Result<llm::ChatChunk, _> = serde_json::from_str(data);
                    let chunk = match parsed { Ok(c) => c, Err(_) => continue, };
                    if let Some(u) = chunk.usage { usage = Some(u); }
                    for ch in chunk.choices {
                        if let Some(t) = ch.delta.content { accum_text.push_str(&t); yield AgentEvent::Text { delta: t }; }
                        if let Some(r) = ch.delta.reasoning_content.or(ch.delta.reasoning) { accum_reasoning.push_str(&r); yield AgentEvent::Reasoning { delta: r }; }
                        if let Some(tcs) = ch.delta.tool_calls {
                            for tc in tcs {
                                let entry = tool_acc.entry(tc.index).or_insert_with(|| ToolAccum { id: tc.id.clone().unwrap_or_default(), name: String::new(), args: String::new() });
                                if let Some(id) = tc.id { if !id.is_empty() { entry.id = id; } }
                                if let Some(f) = tc.function { if let Some(n) = f.name { if !n.is_empty() { entry.name = n; } } if let Some(a) = f.arguments { entry.args.push_str(&a); } }
                            }
                        }
                    }
                }
            }
            for line in buf.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
                let data = if let Some(d) = line.strip_prefix("data: ") { d } else { continue };
                if data == "[DONE]" { continue; }
                if let Ok(chunk) = serde_json::from_str::<llm::ChatChunk>(data) {
                    if let Some(u) = chunk.usage { usage = Some(u); }
                    for ch in chunk.choices {
                        if let Some(t) = ch.delta.content { accum_text.push_str(&t); yield AgentEvent::Text { delta: t }; }
                        if let Some(r) = ch.delta.reasoning_content.or(ch.delta.reasoning) { accum_reasoning.push_str(&r); yield AgentEvent::Reasoning { delta: r }; }
                        if let Some(tcs) = ch.delta.tool_calls {
                            for tc in tcs {
                                let entry = tool_acc.entry(tc.index).or_insert_with(|| ToolAccum { id: tc.id.clone().unwrap_or_default(), name: String::new(), args: String::new() });
                                if let Some(id) = tc.id { if !id.is_empty() { entry.id = id; } }
                                if let Some(f) = tc.function { if let Some(n) = f.name { if !n.is_empty() { entry.name = n; } } if let Some(a) = f.arguments { entry.args.push_str(&a); } }
                            }
                        }
                    }
                }
            }
            if !accum_text.is_empty() { final_text.push_str(&accum_text); yield AgentEvent::TextDone { text: accum_text.clone() }; tracker.record_text(&accum_text); }
            if tool_acc.is_empty() {
                if tracker.is_conversational_goal() {
                    if !accum_text.is_empty() { {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                } }
                    yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                    break;
                }
                tracker.nocall_streak += 1;
                let complete = tracker.looks_complete(&accum_text);
                // State-based completion: phrase alone is not enough for issue-solving; require evidence gate.
                if complete || (tracker.nocall_streak >= MAX_NOCALL_STREAK && tracker.can_complete()) {
                    yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                    break;
                }
                if !accum_text.is_empty() { {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                } }
                continue;
            }
            let mut ordered: Vec<(usize, ToolAccum)> = tool_acc.into_iter().collect();
            ordered.sort_by_key(|(k, _)| *k);
            let tool_names: Vec<String> = ordered.iter().map(|(_, acc)| acc.name.clone()).collect();
            tracker.record_tools(&tool_names);
            let mut tool_results: Vec<(String, String, String, Value)> = Vec::new();
            for (_, acc) in &ordered {
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                yield AgentEvent::ToolStart { name: acc.name.clone(), args: args_val.clone(), id: acc.id.clone() };
            }
            let gate_active = tracker.requires_approval() && !tracker.has_approval();
            let is_ask = is_ask_mode();
            let futs: Vec<_> = ordered.iter().map(|(_, acc)| {
                let name = acc.name.clone();
                let id = acc.id.clone();
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                let is_bash_readonly = name == "bash" && is_readonly_bash(args_val.get("command").and_then(|v| v.as_str()).unwrap_or(""));
                let is_mcp_readonly = name.contains("__") && is_mcp_read(&name);
                let ask_blocked = is_ask && is_mutating_tool(&name) && !is_bash_readonly && !is_mcp_readonly;
                let plan_blocked = gate_active && is_mutating_tool(&name) && !is_plan_exempt_write(&name, &args_val) && !is_bash_readonly && !is_mcp_readonly && !crate::guards::approval::is_auto_accept();
                async move {
                    let start = std::time::Instant::now();
                    let result = if ask_blocked { format!("[ASK BLOCKED] '{}' is blocked — {}. Allowed: read, web_search, grep, find, ls, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, 2>/dev/null), MCP reads. (Shift+Tab to cycle NORM/PLAN/ASK/AUTO)", name, ASK_READONLY_DENY_MSG) } else if plan_blocked { format!("[GATING BLOCKED — plan mode] Mutating tool '{}' is blocked until you complete Phases 1-4 and get explicit user approval via ask_user with '\\u{{2713}} Proceed as proposed'. Call ask_user now to clarify scope/approach. In plan mode only .lean/plans writes + read-only bash (2>/dev/null, pipes) + MCP reads are allowed before approval; all other mutations blocked. (Shift+Tab to cycle NORM/PLAN/ASK/AUTO or /plan to toggle.)", name) } else { crate::tools::execute_tool(&name, args_val.clone()).await };
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    (id, name, result, args_val, elapsed_ms)
                }
            }).collect();
            let results = futures::future::join_all(futs).await;
            for (id, name, result, args_val, elapsed_ms) in results {
                let display = result.find("<<IMAGE:").map_or_else(|| result.clone(), |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()));
                yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                tracker.note_tool_result(&name, &args_val, &result);
                if name == "ask_user" {
                    tracker.note_ask_result(&result);
                }
                tool_results.push((id, name, result, args_val));
            }
            let tool_calls_json: Vec<Value> = ordered.iter().map(|(_, acc)| json!({"id": acc.id, "type": "function", "function": {"name": acc.name, "arguments": acc.args}})).collect();
            {
                    let mut assistant_msg = json!({"role": "assistant", "content": accum_text.clone(), "tool_calls": tool_calls_json.clone()});
                    if !accum_reasoning.is_empty() {
                        assistant_msg["reasoning_content"] = json!(accum_reasoning.clone());
                        assistant_msg["reasoning"] = json!(accum_reasoning.clone());
                    }
                    messages.push(assistant_msg);
                }
            for (id, _name, result, _) in tool_results {
                if let Some(img_start) = result.find("<<IMAGE:") {
                    let meta = result[..img_start].trim_end();
                    let after = &result[img_start + 8..];
                    if let Some(colon_pos) = after.find(':') {
                        let mime = &after[..colon_pos];
                        let b64_raw = after[colon_pos + 1..].trim_end_matches(">>");
                        const MAX_B64_LEN: usize = 68_000;
                        let tool_text = if meta.is_empty() { "Image read follows in next message." } else { meta };
                        let truncated_tool = truncate_for_llm(tool_text);
                        messages.push(json!({"role": "tool", "tool_call_id": id, "content": truncated_tool}));
                        if b64_raw.len() > MAX_B64_LEN {
                            messages.push(json!({"role": "user", "content": format!("[image from tool {} omitted — {:.1} KB too large, max 50KB]", _name, b64_raw.len() as f64 * 0.75 / 1024.0)}));
                        } else {
                            let is_valid = b64_raw.chars().all(|c| c.is_ascii_alphanumeric() || c=='+' || c=='/' || c=='=');
                            if !is_valid {
                                messages.push(json!({"role": "user", "content": "[image data invalid — omitted]"}));
                            } else {
                                messages.push(json!({"role": "user", "content": [{"type":"text","text": format!("[image from {}: {}]", _name, mime)},{"type":"image_url","image_url":{"url": format!("data:{};base64,{}", mime, b64_raw)}}]}));
                            }
                        }
                        continue;
                    }
                }
                let truncated = truncate_for_llm(&result);
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": truncated}));
            }
            drop_orphaned_tool_outputs(&mut messages);
            let _ = usage;
        }
        yield AgentEvent::Done { text: final_text, history: messages.clone() };
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;
    const BASE_BUDGET: usize = 3600;
    const TOTAL_BUDGET: usize = 12000;

    #[test]
    fn system_prompt_within_base_budget() {
        assert!(
            SYSTEM_PROMPT.len() <= BASE_BUDGET,
            "SYSTEM_PROMPT len {} > {}",
            SYSTEM_PROMPT.len(),
            BASE_BUDGET
        );
    }

    #[tokio::test]
    async fn built_prompt_within_total_budget() {
        let built = build_system_prompt().await;
        assert!(
            built.len() <= TOTAL_BUDGET,
            "built prompt len {} > {}",
            built.len(),
            TOTAL_BUDGET
        );
    }

    #[tokio::test]
    async fn built_prompt_keeps_base_prompt_intact() {
        let built = build_system_prompt().await;
        assert!(
            built.starts_with(SYSTEM_PROMPT),
            "base prompt was altered or truncated"
        );
    }

    #[test]
    fn truncate_helpers_are_utf8_safe() {
        let s = "héllo wörld 😀 ending";
        let truncated = truncate_str(s, 5);
        assert_eq!(truncated.chars().count(), 5);
        assert!(truncated.ends_with('…'));

        let chars = truncate_chars(s, 5);
        assert_eq!(chars.chars().count(), 5);

        assert_eq!(truncate_str(s, 1000), s);
        assert!(truncate_str(s, 0).is_empty());
        assert!(truncate_to_bytes(s, 10).len() <= 10);
    }
}

#[cfg(test)]
mod behavior_tests {
    use super::*;

    #[test]
    fn conversational_goals_detected() {
        for goal in [
            "hi",
            "hello",
            "hey",
            "thanks",
            "thank you",
            "how are you?",
            "Hi!",
            "Hey there",
        ] {
            assert!(
                PlanTracker::new(goal).is_conversational_goal(),
                "{goal:?} should be conversational"
            );
        }
    }

    #[test]
    fn task_goals_are_not_conversational() {
        for goal in [
            "read README.md",
            "build a simple UI",
            "thanks, now fix the login bug",
            "please read README.md",
            "run the tests",
        ] {
            assert!(
                !PlanTracker::new(goal).is_conversational_goal(),
                "{goal:?} should be a task"
            );
        }
    }

    #[test]
    fn completion_requires_terminal_signal() {
        let tracker = PlanTracker::new("read README.md");
        assert!(tracker.looks_complete("All done."));
        assert!(tracker.looks_complete("Here's what I did: I read the file."));
        assert!(!tracker.looks_complete("Let me look at the code."));
        assert!(!tracker.looks_complete("Summary: I'll now edit the file."));
        assert!(!tracker.looks_complete("Next step: update the parser."));
    }

    #[test]
    fn focus_context_carries_goal_and_step() {
        let tracker = PlanTracker::new("fix the parser");
        let focus = tracker.focus_context(2);
        assert!(focus.contains("fix the parser"));
        assert!(focus.contains("Step 2"));
    }

    #[test]
    fn focus_context_flags_repeated_no_tool_steps() {
        let mut tracker = PlanTracker::new("fix the parser");
        tracker.nocall_streak = 1;
        let focus = tracker.focus_context(2);
        assert!(focus.contains(&format!("attempt 1/{}", MAX_NOCALL_STREAK)));
    }

    #[test]
    fn prune_context_messages_drops_injected_context_only() {
        let messages = vec![
            json!({"role": "system", "content": SYSTEM_PROMPT}),
            json!({"role": "system", "content": "[Focus]\nGoal: x"}),
            json!({"role": "system", "content": "[Continue] attempt 1/3"}),
            json!({"role": "system", "content": "Recalled memories: ..."}),
            json!({"role": "user", "content": "hi"}),
        ];
        let pruned = prune_context_messages(&messages);
        assert_eq!(pruned.len(), 3);
        assert!(pruned[1]["content"]
            .as_str()
            .unwrap()
            .starts_with("Recalled memories"));
        assert_eq!(pruned[2]["content"], "hi");
    }
}
