use crate::llm::{self, Client};
use crate::skills;
use futures::Stream;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const SYSTEM_PROMPT: &str = "You are an autonomous coding agent. You must complete tasks fully before stopping.\n\n\
## CRITICAL RULES (follow these every time)\n\n\
1. **DO NOT STOP until the task is complete.** You are an autonomous agent. When given a task, you work through it step by step using tools until it is fully done. Never give up, never ask the user to do it themselves, never stop early.\n\n\
2. **ALWAYS check skills first.** Before starting any task, look at the available skills list. If any skill matches your task, call read_skill to load its instructions, then follow them. Skills contain proven workflows -- use them.\n\n\
3. **ALWAYS search memory first.** At the start of a task, use search_memory to find relevant context from past sessions. If you learn something important during the task, remember it with the remember tool.\n\n\
4. **Verify your work.** After making changes, run relevant commands (build, test, lint, etc.) to confirm they work. Never assume success.\n\n\
5. **Use tools liberally.** Read files before editing them. Use bash to test. Use web_search if you need information. The more tools you use, the better your work.\n\n\
## How to approach any task\n\n\
When you receive a task, your FIRST response must:\n\
1. Search memory for relevant context\n\
2. Check if any skill applies (call read_skill if so)\n\
3. State the goal in one sentence\n\
4. List concrete steps (read file, make change, verify, etc)\n\
5. Begin executing immediately\n\n\
You will receive a focus context injection before each LLM call that reminds you of your goal, progress, and next step. **Always check this before responding.** If the focus context says you should be doing something, do it.\n\n\
## Staying on track\n\n\
- **Always check the focus context** at the start of each response.\n\
- **Never repeat a tool call** that already succeeded.\n\
- If a tool call failed, diagnose the error and try a different approach.\n\
- After completing all steps, give a clear summary and explicitly state you are done.\n\
- If you find yourself unsure what to do next, re-read the original task, check your progress, and figure it out.\n\n\
## Directory confinement (HARD WALL)\n\n\
You are confined to the project CWD (shown in footer/session). Any read/write/edit/bash that touches a path outside CWD will be BLOCKED and require user approval [a]/[A]. The user chose hard-wall mode: stay inside unless they explicitly asked to go outside.\n\n\
## Tool usage\n\n\
1. **Understand first.** Before changing anything, read the relevant files. Never edit a file you haven't read.\n\
2. **Plan minimally.** Decide the smallest set of changes that solves the problem.\n\
3. **Act with tools.** Use read_file, bash, edit_file, write_file. Use web_search if needed.\n\
4. **When it fails, diagnose.** Read error messages. Re-read code. Try a different approach.\n\
5. **Verify after.** Run build, test, or relevant commands to confirm. Don't assume success.\n\n\
## Memory\n\n\
You have persistent memory. Use it.\n\n\
- `remember` -- Store important facts, corrections, procedures, user preferences\n\
- `search_memory` -- Search memories by keyword (DO THIS at task start)\n\
- `recall_memory` -- List most recent memories\n\
- `list_memories` -- List by tag\n\
- `forget_memory` -- Delete by id\n\
- `consolidate_memory` -- Deduplicate Jaccard>0.75, bound buffer\n\
- `memory_stats` -- Show stats by category/scope\n\n\
Categories: fact, preference, correction, procedure.\n\
Scopes: global (always recalled), project (current codebase only).\n\n\
**When you learn something important, remember it. When starting a task, search memory.**\n\n\
## Skills\n\n\
You have a skill system. Skills are markdown files containing proven workflows and instructions.\n\n\
Available skills are listed at the top of this prompt. When a task matches a skill:\n\
1. Call read_skill with the skill name\n\
2. Read the returned instructions carefully\n\
3. Follow the skill's workflow\n\n\
**Do not ignore skills. They exist to make you better at your job.**\n\n\
## Final reminder\n\n\
**You are an autonomous agent. You do the work. You don't stop until it's done. You use skills. You use memory. You verify your work.**";

pub async fn build_system_prompt() -> String {
    let catalog = skills::get_skill_catalog().await;
    let cwd = crate::dir_guard::project_root().display().to_string();
    let dir_note = if crate::dir_guard::is_disabled() {
        String::new()
    } else {
        format!("\n\n## Confinement\nYou are confined to CWD: `{}`. Any file or bash path outside this dir will be blocked until the user approves ([a]/[A]). Do not try to bypass with `../` or absolute paths unless the user asked to go outside.", cwd)
    };
    format!(
        "{}\n\n## Available Skills\nThese skills contain proven workflows for specific tasks. You MUST check if any skill matches your current task.\n{}\n\n**If a skill matches your task, call read_skill immediately. Then follow the skill's instructions.** You may call multiple read_skill in one step if needed.{}",
        SYSTEM_PROMPT, catalog, dir_note
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
    ToolResult { name: String, result: String, id: String, elapsed_ms: u64 },
    Step { n: usize },
    Done { text: String, history: Vec<Value> },
}

struct ToolAccum {
    id: String,
    name: String,
    args: String,
}

/// Tracks the agent's goal, progress, and tool usage across steps.
/// Injected into the conversation as a focus context so the LLM stays on task.
struct PlanTracker {
    goal: String,
    steps_done: Vec<String>,
    last_tools: Vec<String>,
    /// How many consecutive steps the LLM produced text but no tool calls.
    /// If this exceeds a threshold, we re-prompt instead of stopping.
    nocall_streak: usize,
}

/// Maximum consecutive text-only (no tool call) responses before we force-stop.
const MAX_NOCALL_STREAK: usize = 3;

impl PlanTracker {
    fn new(goal: &str) -> Self {
        Self {
            goal: goal.to_string(),
            steps_done: Vec::new(),
            last_tools: Vec::new(),
            nocall_streak: 0,
        }
    }

    /// Check whether the assistant's text output indicates task completion.
    /// We require multiple completion signals to avoid false positives on
    /// intermediate messages like "I've successfully read the file."
    fn looks_complete(text: &str) -> bool {
        let lower = text.to_lowercase();
        // Heuristic: if the assistant produces a summary-like message with no tools,
        // treat it as a completion signal.
        let signals = [
            "here's what i did",
            "here is what i did",
            "summary:",
            "in summary",
            "that completes",
            "all done",
            "task complete",
            "i've finished",
            "i have finished",
        ];
        let matches = signals.iter().filter(|s| lower.contains(*s)).count();
        // Require at least 2 signal matches, or 1 signal plus a length
        // indicator (>= 100 chars means it's a real summary, not a blip)
        matches >= 2 || (matches >= 1 && text.len() >= 100)
    }

    /// Build a focus injection message to insert into the conversation.
    fn focus_context(&self, step: usize) -> String {
        let mut out = String::from("[FOCUS CONTEXT -- READ THIS BEFORE RESPONDING]\n");
        out.push_str(&format!("Goal: {}\n", self.goal));
        if !self.steps_done.is_empty() {
            out.push_str(&format!(
                "Progress so far ({} items):\n",
                self.steps_done.len()
            ));
            for (i, s) in self.steps_done.iter().enumerate() {
                out.push_str(&format!("  {}. {}\n", i + 1, s));
            }
        } else {
            out.push_str("Progress so far: starting\n");
        }
        if !self.last_tools.is_empty() {
            out.push_str(&format!(
                "Last tool calls: {}\n",
                self.last_tools.join(", ")
            ));
        }
        out.push_str(&format!(
            "Current step: {}. If your plan is complete, summarize what you did and stop.\n",
            step
        ));
        out.push_str("Do NOT repeat actions already listed in progress. Stay focused on the goal.");
        out
    }

    /// Build a re-prompt message injected when the LLM stops without tools.
    fn continue_prompt(&self) -> String {
        format!(
            "[CONTINUATION REQUIRED]\n\
            You stopped without using tools. The goal has NOT been achieved yet:\n\
            {}\n\
            Keep working. Use your tools to make progress. Do NOT stop until the task is fully complete. \
            If you need to verify your work, use bash to run tests or checks. \
            If you already finished, provide a final summary and explicitly state 'All done.'",
            self.goal
        )
    }

    /// Update tracker with completed tool names.
    fn record_tools(&mut self, tool_names: &[String]) {
        self.nocall_streak = 0; // reset streak when tools are used
        self.last_tools = tool_names.to_vec();
        for name in tool_names {
            self.steps_done.push(format!("called {}", name));
        }
    }

    /// Update tracker with the assistant's text output (concise summary of what happened).
    fn record_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            // Keep it short: first sentence or first 200 chars
            let summary = if let Some(period) = trimmed.find('.') {
                if period < 200 {
                    trimmed[..period + 1].to_string()
                } else {
                    trimmed.chars().take(200).collect()
                }
            } else {
                trimmed.chars().take(200).collect()
            };
            self.steps_done.push(summary);
        }
    }
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
        let client = Client::from_env();
        let system = build_system_prompt().await;
        let mut messages: Vec<Value> = vec![
            json!({"role": "system", "content": system}),
        ];
        // Inject exact llm history if available (tool_calls preserved), else fallback
        // history is already Vec<Value> with proper roles (user/assistant/tool/system)
        // Truncate to last 20 entries and 3000 chars per text content to stay in context
        let hist_slice = if history.len() > 20 { &history[history.len()-20..] } else { &history[..] };
        for v in hist_slice {
            // Clone and truncate text content if needed
            let mut val = v.clone();
            if let Some(content) = val.get("content") {
                if let Some(s) = content.as_str() {
                    if s.len() > 3000 {
                        val["content"] = json!(format!("{}… [truncated]", &s[..3000]));
                    }
                }
            }
            messages.push(val);
        }
        messages.push(json!({"role": "user", "content": &user_prompt}));

        let mut final_text = String::new();
        let mut tracker = PlanTracker::new(&user_prompt);

        for step in 0..max_steps {
            yield AgentEvent::Step { n: step + 1 };

            // ── Autorecall: search memories using recent conversation context ──
            if step > 0 {
                let context: String = messages
                    .iter()
                    .rev()
                    .take(4)
                    .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                    .collect::<Vec<&str>>()
                    .join(" ");
                if let Some(memory_note) = crate::memory::autorecall(&context) {
                    let recall_msg = json!({"role": "system", "content": memory_note});
                    if messages.len() > 1
                        && messages[1].get("role").and_then(|r| r.as_str()) == Some("system")
                        && messages[1].get("content").and_then(|c| c.as_str())
                            .map(|c| c.starts_with("Recalled memories"))
                            .unwrap_or(false)
                    {
                        messages[1] = recall_msg;
                    } else {
                        messages.insert(1, recall_msg);
                    }
                }
            }

            // ── Inject focus context so the LLM stays on task ──
            // Position: right before the last assistant/tool messages, so the
            // LLM sees "here's what you're doing" immediately before deciding
            // what to do next. We insert at a stable position: index 2
            // (after system + optional recall, before user and rest).
            let focus_msg = json!({"role": "system", "content": tracker.focus_context(step + 1)});
            // Remove any previous focus injection (look for the marker)
            let marker = "[FOCUS CONTEXT";
            let mut removed_old = false;
            for i in (2..messages.len()).rev() {
                if messages[i].get("role").and_then(|r| r.as_str()) == Some("system")
                    && messages[i].get("content").and_then(|c| c.as_str())
                        .map(|c| c.starts_with(marker))
                        .unwrap_or(false)
                {
                    messages.remove(i);
                    removed_old = true;
                    break;
                }
            }
            // Insert at position 2 (or wherever it was before)
            let insert_at = if removed_old { 2 } else { 2 };
            if insert_at < messages.len() {
                messages.insert(insert_at, focus_msg);
            } else {
                messages.push(focus_msg);
            }

            // ── Build request ──
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

            // ── SSE parsing ──
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
                // Track what the assistant said for progress
                tracker.record_text(&accum_text);
            }

            if tool_acc.is_empty() {
                tracker.nocall_streak += 1;
                let complete = PlanTracker::looks_complete(&accum_text);
                if complete || tracker.nocall_streak >= MAX_NOCALL_STREAK {
                    yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                    break;
                }
                // Push the assistant's text so the model sees its own response
                if !accum_text.is_empty() {
                    messages.push(json!({"role": "assistant", "content": accum_text}));
                }
                // Re-prompt: inject a continuation message and loop again
                let continue_msg = json!({"role": "system", "content": tracker.continue_prompt()});
                messages.push(continue_msg);
                continue;
            }

            // ── Prepare tool calls in index order ──
            let mut ordered: Vec<(usize, ToolAccum)> = tool_acc.into_iter().collect();
            ordered.sort_by_key(|(k, _)| *k);

            // Track which tools are being called
            let tool_names: Vec<String> = ordered.iter().map(|(_, acc)| acc.name.clone()).collect();
            tracker.record_tools(&tool_names);

            let mut tool_results: Vec<(String, String, String, Value)> = Vec::new();
            // Yield tool_start for all
            for (_, acc) in &ordered {
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                yield AgentEvent::ToolStart { name: acc.name.clone(), args: args_val.clone(), id: acc.id.clone() };
            }
            // Execute in parallel — per-tool wall-clock timing
            let futs: Vec<_> = ordered.iter().map(|(_, acc)| {
                let name = acc.name.clone();
                let id = acc.id.clone();
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                async move {
                    let start = std::time::Instant::now();
                    let result = crate::tools::execute_tool(&name, args_val.clone()).await;
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    (id, name, result, args_val, elapsed_ms)
                }
            }).collect();
            let results = futures::future::join_all(futs).await;
            for (id, name, result, args_val, elapsed_ms) in results {
                let display = result.find("<<IMAGE:").map_or_else(
                    || result.clone(),
                    |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()),
                );
                yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                tool_results.push((id, name, result, args_val));
            }

            // ── Append assistant tool_calls to messages ──
            let tool_calls_json: Vec<Value> = ordered.iter().map(|(_, acc)| {
                json!({
                    "id": acc.id,
                    "type": "function",
                    "function": {"name": acc.name, "arguments": acc.args}
                })
            }).collect();
            messages.push(json!({"role": "assistant", "content": accum_text, "tool_calls": tool_calls_json}));
            for (id, _name, result, _) in tool_results {
                let truncated = truncate_for_llm(&result);
                // Build the message content: multimodal if the result contains an image marker
                let content: Value = if let Some(img_start) = result.find("<<IMAGE:") {
                    let meta_line = result[..img_start].trim_end();
                    let after_marker = &result[img_start + 8..];
                    if let Some(colon_pos) = after_marker.find(':') {
                        let mime = &after_marker[..colon_pos];
                        let b64 = after_marker[colon_pos + 1..].trim_end_matches(">>");
                        json!([
                            {"type": "text", "text": meta_line},
                            {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime, b64)}}
                        ])
                    } else {
                        json!(truncated)
                    }
                } else {
                    json!(truncated)
                };
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
            }
            let _ = usage;
        }
        yield AgentEvent::Done { text: final_text, history: messages.clone() };
    }
}
