use crate::llm::{self, Client};
use crate::skills;
use futures::Stream;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const SYSTEM_PROMPT: &str = "You are a lean coding assistant. Be helpful, precise, and concise.\n\n\
## How to approach any task\n\n\
1. **Understand first.** Before changing anything, read the relevant files to understand the existing structure, style, and conventions. Never edit a file you haven't read.\n\n\
2. **Plan minimally.** Decide the smallest set of changes that solves the problem. One function, one file, one fix at a time. Avoid large rewrites when a small edit will do.\n\n\
3. **Act with tools.** Use read_file to inspect, bash for inspection and testing, edit_file/write_file for changes. Use bash to verify your changes compile or run correctly.\n\n\
4. **When it fails, diagnose.** Read error messages carefully. Re-read the code. Try a different approach. Do not repeat the same failing change. If a tool call fails, check the output, fix the cause, and retry.\n\n\
5. **Verify after.** After making changes, run a build, test, or relevant command to confirm it works. Don't assume success.\n\n\
## Rules\n\n\
- Be fully autonomous until the task is done.\n\
- Prefer read_file before edit_file.\n\
- Use bash for inspection, building, and testing — not just for running the user's request.\n\
- Make the smallest change that works.\n\
- If something is unclear, gather more context from the codebase before guessing.\n\n\
## Memory\n\n\
You have persistent memory tools. Use them to remember important facts, user preferences, corrections, and procedures across sessions.\n\n\
- `remember` -- Store something important (content, category, tags, scope)\n\
- `search_memory` -- Search memories by keyword\n\
- `recall_memory` -- List most recent memories\n\
- `list_memories` -- List memories filtered by tag\n\
- `forget_memory` -- Delete a memory by id\n\n\
Categories: fact, preference, correction, procedure.\n\
Scopes: global (always recalled), project (current codebase only).\n\n\
When you learn something important about the user or project, remember it automatically. When starting a task, search memory for relevant context.";

pub async fn build_system_prompt() -> String {
    let catalog = skills::get_skill_catalog().await;
    format!(
        "{}\n\nAvailable skills (pi-style .md from ~/.agents/skills + project ./skills). Load one or multiple with read_skill when relevant — read_skill returns full SKILL.md instructions to follow:\n{}\n\nWhen a task matches a skill, call read_skill. You may call multiple read_skill in one step if needed.",
        SYSTEM_PROMPT, catalog
    )
}

const MAX_TOOL_OUTPUT_FOR_LLM: usize = 2000;

fn truncate_for_llm(s: &str) -> String {
    if s.len() <= MAX_TOOL_OUTPUT_FOR_LLM {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX_TOOL_OUTPUT_FOR_LLM).collect();
    let remaining = s.chars().count() - MAX_TOOL_OUTPUT_FOR_LLM;
    format!(
        "{}… [truncated {} chars for LLM, full shown in TUI]",
        truncated,
        remaining
    )
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Text { delta: String },
    Reasoning { delta: String },
    TextDone { text: String },
    ToolStart { name: String, args: Value, id: String },
    ToolResult { name: String, result: String, id: String },
    Step { n: usize },
    Done { text: String },
}

struct ToolAccum {
    id: String,
    name: String,
    args: String,
}

pub fn run_agent(
    user_prompt: String,
    model: String,
    max_steps: usize,
) -> impl Stream<Item = AgentEvent> {
    async_stream::stream! {
        let client = Client::from_env();
        let system = build_system_prompt().await;
        let mut messages: Vec<Value> = vec![
            json!({"role": "system", "content": system}),
            json!({"role": "user", "content": user_prompt}),
        ];

        let mut final_text = String::new();

        for step in 0..max_steps {
            yield AgentEvent::Step { n: step + 1 };

            // Autorecall: search memories using recent conversation context
            if step > 0 {
                // Build context from last few messages
                let context: String = messages
                    .iter()
                    .rev()
                    .take(4)
                    .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                    .collect::<Vec<&str>>()
                    .join(" ");
                if let Some(memory_note) = crate::memory::autorecall(&context) {
                    messages.insert(1, json!({"role": "system", "content": memory_note}));
                }
            }

            // Build request
            let body = json!({
                "model": model,
                "messages": messages,
                "tools": llm::tool_definitions(),
                "tool_choice": "auto",
                "stream": true
            });

            let resp = match client.http.post(client.chat_url())
                .header("Authorization", format!("Bearer {}", client.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send().await {
                    Ok(r) => r,
                    Err(e) => {
                        yield AgentEvent::Text { delta: format!("\n[LLM error: {}]", e) };
                        break;
                    }
                };

            if !resp.status().is_success() {
                let txt = resp.text().await.unwrap_or_default();
                yield AgentEvent::Text { delta: format!("\n[LLM HTTP error: {}]", txt) };
                break;
            }

            // SSE parsing
            let mut accum_text = String::new();
            let mut accum_reasoning = String::new();
            let mut tool_acc: HashMap<usize, ToolAccum> = HashMap::new();
            let mut usage: Option<llm::Usage> = None;

            let mut stream = resp.bytes_stream();
            use futures::StreamExt;
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield AgentEvent::Text { delta: format!("\n[stream error: {}]", e) };
                        break;
                    }
                };
                let text = String::from_utf8_lossy(&bytes);
                buf.push_str(&text);
                // process complete lines
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf.drain(..pos+1);
                    if line.is_empty() { continue; }
                    let data = if let Some(d) = line.strip_prefix("data: ") { d } else { continue };
                    if data == "[DONE]" { break; }
                    let parsed: Result<llm::ChatChunk, _> = serde_json::from_str(data);
                    let chunk = match parsed {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    if let Some(u) = chunk.usage { usage = Some(u); }
                    for ch in chunk.choices {
                        if let Some(t) = ch.delta.content {
                            accum_text.push_str(&t);
                            yield AgentEvent::Text { delta: t };
                        }
                        if let Some(r) = ch.delta.reasoning_content.or(ch.delta.reasoning) {
                            accum_reasoning.push_str(&r);
                            yield AgentEvent::Reasoning { delta: r };
                        }
                        if let Some(tcs) = ch.delta.tool_calls {
                            for tc in tcs {
                                let entry = tool_acc.entry(tc.index).or_insert_with(|| ToolAccum { id: tc.id.clone().unwrap_or_default(), name: String::new(), args: String::new() });
                                if let Some(id) = tc.id { if !id.is_empty() { entry.id = id; } }
                                if let Some(f) = tc.function {
                                    if let Some(n) = f.name { if !n.is_empty() { entry.name = n; } }
                                    if let Some(a) = f.arguments { entry.args.push_str(&a); }
                                }
                            }
                        }
                    }
                }
            }
            // also handle leftover buf
            for line in buf.lines() {
                let line = line.trim();
                if line.is_empty() { continue; }
                let data = if let Some(d) = line.strip_prefix("data: ") { d } else { continue };
                if data == "[DONE]" { continue; }
                if let Ok(chunk) = serde_json::from_str::<llm::ChatChunk>(data) {
                    if let Some(u) = chunk.usage { usage = Some(u); }
                    for ch in chunk.choices {
                        if let Some(t) = ch.delta.content {
                            accum_text.push_str(&t);
                            yield AgentEvent::Text { delta: t };
                        }
                        if let Some(r) = ch.delta.reasoning_content.or(ch.delta.reasoning) {
                            accum_reasoning.push_str(&r);
                            yield AgentEvent::Reasoning { delta: r };
                        }
                        if let Some(tcs) = ch.delta.tool_calls {
                            for tc in tcs {
                                let entry = tool_acc.entry(tc.index).or_insert_with(|| ToolAccum { id: tc.id.clone().unwrap_or_default(), name: String::new(), args: String::new() });
                                if let Some(id) = tc.id { if !id.is_empty() { entry.id = id; } }
                                if let Some(f) = tc.function {
                                    if let Some(n) = f.name { if !n.is_empty() { entry.name = n; } }
                                    if let Some(a) = f.arguments { entry.args.push_str(&a); }
                                }
                            }
                        }
                    }
                }
            }

            if !accum_reasoning.is_empty() {
                // reasoning already yielded
            }
            if !accum_text.is_empty() {
                final_text.push_str(&accum_text);
                yield AgentEvent::TextDone { text: accum_text.clone() };
            }

            if tool_acc.is_empty() {
                yield AgentEvent::Done { text: final_text.clone() };
                break;
            }

            // Prepare tool calls in index order
            let mut ordered: Vec<(usize, ToolAccum)> = tool_acc.into_iter().collect();
            ordered.sort_by_key(|(k, _)| *k);
            let mut tool_results: Vec<(String, String, String, Value)> = Vec::new(); // id, name, result, args
            // Yield tool_start for all
            for (_, acc) in &ordered {
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                yield AgentEvent::ToolStart { name: acc.name.clone(), args: args_val.clone(), id: acc.id.clone() };
            }
            // Execute in parallel
            let futs: Vec<_> = ordered.iter().map(|(_, acc)| {
                let name = acc.name.clone();
                let id = acc.id.clone();
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                async move {
                    let result = crate::tools::execute_tool(&name, args_val.clone()).await;
                    (id, name, result, args_val)
                }
            }).collect();
            let results = futures::future::join_all(futs).await;
            for (id, name, result, args_val) in results {
                let display = result.clone();
                yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone() };
                tool_results.push((id, name, result, args_val));
            }

            // Append assistant tool_calls to messages
            let tool_calls_json: Vec<Value> = ordered.iter().map(|(_, acc)| {
                json!({
                    "id": acc.id,
                    "type": "function",
                    "function": {"name": acc.name, "arguments": acc.args}
                })
            }).collect();
            messages.push(json!({"role": "assistant", "content": accum_text, "tool_calls": tool_calls_json}));
            for (id, name, result, _) in tool_results {
                let truncated = truncate_for_llm(&result);
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": truncated}));
                // CWD update: if bash did cd, try to track
                if name == "bash" {
                    // heuristic: if command contains `cd `, try to update current_dir
                    // actual cd in subshell doesn't affect parent, so footer CWD stays launch dir — document limitation
                }
            }
            // Also push usage to messages? No, just continue loop. Footer can use usage if needed via separate channel.
            let _ = usage;
        }
        yield AgentEvent::Done { text: final_text };
    }
}
