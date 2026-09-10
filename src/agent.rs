use crate::llm::{self, Client};
use crate::skills;
use futures::Stream;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const SYSTEM_PROMPT: &str = "You are lean — a lightweight, friendly coding assistant.\n\n\
## Conversation vs Task — READ FIRST\n\n\
- **Greeting / smalltalk / thanks / \"hi\" / \"how are you\" with NO explicit task → reply warmly in 1-2 sentences, offer help, and STOP.** Do NOT call tools, do NOT search memory, do NOT list steps, do NOT invent work. A single friendly response is complete. Replying 3 times to \"Hi\" is a bug — reply once and stop.\n\
- **Only enter TASK MODE** when the user explicitly asks you to do something (write/edit code, research, build UI, fix bug, manage configs, run commands, etc.).\n\n\
## CRITICAL RULES (only in TASK MODE)\n\n\
1. **DO NOT STOP until the task is complete.** In task mode you are autonomous: work step by step with tools until fully done. Never give up, never ask the user to do it themselves, never stop early.\n\n\
2. **ALWAYS check skills first.** Before starting any task, look at the available skills list. If any skill matches your task, call read_skill to load its instructions, then follow them. Skills contain proven workflows -- use them.\n\n\
3. **ALWAYS search memory first.** At the start of a task, use search_memory to find relevant context from past sessions. If you learn something important during the task, remember it with the remember tool.\n\n\
4. **Verify your work.** After making changes, run relevant commands (build, test, lint, etc.) to confirm they work. Never assume success.\n\n\
5. **Use tools liberally.** Read files before editing them. Use bash to test. Use web_search if you need information. The more tools you use, the better your work.\n\n\
## How to approach a task (TASK MODE only)\n\n\
When you receive a task, your FIRST response must:\n\
1. Search memory for relevant context\n\
2. Check if any skill applies (call read_skill if so)\n\
3. State the goal in one sentence\n\
4. List concrete steps (read file, make change, verify, etc)\n\
5. Begin executing immediately\n\n\
You will receive a focus context injection before each LLM call that reminds you of your goal, progress, and next step. **In task mode, always check this before responding.** If the focus context says you should be doing something, do it.\n\n\
## Staying on track\n\n\
- **In task mode, always check the focus context** at the start of each response.\n\
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
**Conversation: one friendly reply and stop. Task mode: you do the work — don't stop until it's done. You use skills. You use memory. You verify your work.**";

pub async fn build_system_prompt() -> String {
    let catalog = skills::get_skill_catalog().await;
    let cwd = crate::dir_guard::project_root().display().to_string();
    let dir_note = if crate::dir_guard::is_disabled() {
        String::new()
    } else {
        format!("\n\n## Confinement\nYou are confined to CWD: `{}`. Any file or bash path outside this dir will be blocked until the user approves ([a]/[A]). Do not try to bypass with `../` or absolute paths unless the user asked to go outside.", cwd)
    };
    let mcp_snap = crate::mcp::snapshot();
    let mcp_note = if mcp_snap.is_empty() {
        "\n\n## MCP\nNo MCP servers configured. To enable MCP, create ~/.lean/mcp.json or .lean/mcp.json (see mcp.json.example).".to_string()
    } else {
        let mut s = String::from("\n\n## MCP Servers (tools are namespaced server__tool, every call requires approval [a]/[A])\n");
        for srv in &mcp_snap {
            let status = srv.status.as_str();
            s.push_str(&format!("- {} [{}] ({} tools)", srv.name, status, srv.tools.len()));
            if !srv.tools.is_empty() {
                let names: Vec<String> = srv.tools.iter().take(5).map(|t| t.name.clone()).collect();
                s.push_str(&format!(": {}", names.join(", ")));
                if srv.tools.len() > 5 { s.push_str(&format!(" +{} more", srv.tools.len()-5)); }
            }
            if let Some(err) = &srv.error_detail {
                s.push_str(&format!(" — error: {}", err));
            }
            s.push('\n');
        }
        s.push_str("\nUse MCP tools like you use native tools. They are merged into the tool list as server__tool. Check /mcp for status.");
        s
    };
    format!(
        "{}\n\n## Available Skills\nThese skills contain proven workflows for specific tasks. You MUST check if any skill matches your current task.\n{}\n\n**If a skill matches your task, call read_skill immediately. Then follow the skill's instructions.** You may call multiple read_skill in one step if needed.{}{}",
        SYSTEM_PROMPT, catalog, dir_note, mcp_note
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

struct PlanTracker {
    goal: String,
    steps_done: Vec<String>,
    last_tools: Vec<String>,
    nocall_streak: usize,
}

const MAX_NOCALL_STREAK: usize = 3;

impl PlanTracker {
    fn new(goal: &str) -> Self {
        Self { goal: goal.to_string(), steps_done: Vec::new(), last_tools: Vec::new(), nocall_streak: 0 }
    }
    fn looks_complete(text: &str) -> bool {
        let lower = text.to_lowercase();
        let signals = ["here's what i did","here is what i did","summary:","in summary","that completes","all done","task complete","i've finished","i have finished"];
        let matches = signals.iter().filter(|s| lower.contains(*s)).count();
        matches >= 2 || (matches >= 1 && text.len() >= 100)
    }
    fn is_conversational_goal(&self) -> bool {
        let g = self.goal.trim().to_lowercase();
        let stripped = g.trim_matches(|c: char| c == '!' || c == '.' || c == ',' || c == '?' || c == '\'' || c == '"').trim();
        let conversational_exact = ["hi","hello","hey","hi there","hello there","hey there","thanks","thank you","thanks!","thank you!","yo","sup","howdy","hola","how are you","how are you?","hey!","hello!","hi!"];
        if conversational_exact.contains(&stripped) { return true; }
        if stripped.len() < 30 {
            let has_task_verb = ["write","create","fix","build","edit","read","search","make","add","update","implement","explain","help with","can you","could you","please"].iter().any(|v| stripped.contains(v));
            if !has_task_verb {
                let greet_prefixes = ["hi ","hello ","hey ","thanks ","thank you "];
                if greet_prefixes.iter().any(|p| stripped.starts_with(p)) { return true; }
                if stripped.split_whitespace().count() <= 3 && !has_task_verb {
                    if ["hi","hello","hey"].iter().any(|w| stripped.contains(w)) { return true; }
                }
            }
        }
        false
    }
    fn focus_context(&self, step: usize) -> String {
        if self.is_conversational_goal() {
            return format!("[FOCUS CONTEXT — conversational turn]\nGoal: \"{}\" — this is smalltalk/greeting, NOT a task. Respond warmly in 1-2 sentences and STOP. Do NOT call tools, do NOT search memory, do NOT list steps.\nCurrent step: {}. No focus tracking needed.", self.goal, step);
        }
        let mut out = String::from("[FOCUS CONTEXT -- READ THIS BEFORE RESPONDING]\n");
        out.push_str(&format!("Goal: {}\n", self.goal));
        if !self.steps_done.is_empty() {
            out.push_str(&format!("Progress so far ({} items):\n", self.steps_done.len()));
            for (i, s) in self.steps_done.iter().enumerate() { out.push_str(&format!("  {}. {}\n", i+1, s)); }
        } else { out.push_str("Progress so far: starting\n"); }
        if !self.last_tools.is_empty() { out.push_str(&format!("Last tool calls: {}\n", self.last_tools.join(", "))); }
        out.push_str(&format!("Current step: {}. If your plan is complete, summarize what you did and stop.\n", step));
        out.push_str("Do NOT repeat actions already listed in progress. Stay focused on the goal.");
        out
    }
    fn continue_prompt(&self) -> String {
        format!("[CONTINUATION REQUIRED]\nYou stopped without using tools. The goal has NOT been achieved yet:\n{}\nKeep working. Use your tools to make progress. Do NOT stop until the task is fully complete. If you need to verify your work, use bash to run tests or checks. If you already finished, provide a final summary and explicitly state 'All done.'", self.goal)
    }
    fn record_tools(&mut self, tool_names: &[String]) {
        self.nocall_streak = 0;
        self.last_tools = tool_names.to_vec();
        for name in tool_names { self.steps_done.push(format!("called {}", name)); }
    }
    fn record_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let summary = if let Some(period) = trimmed.find('.') {
                if period < 200 { trimmed[..period+1].to_string() } else { trimmed.chars().take(200).collect() }
            } else { trimmed.chars().take(200).collect() };
            self.steps_done.push(summary);
        }
    }
}

pub fn run_agent(user_prompt: String, model: String, max_steps: usize) -> impl Stream<Item = AgentEvent> {
    run_agent_with_history(user_prompt, model, max_steps, Vec::new())
}

pub fn run_agent_with_history(user_prompt: String, model: String, max_steps: usize, history: Vec<Value>) -> impl Stream<Item = AgentEvent> {
    async_stream::stream! {
        let resolved = match crate::models::resolve(Some(&model)) {
            Ok(r) => r,
            Err(e) => {
                yield AgentEvent::Text { delta: format!("\n[model resolve error: {}]", e) };
                yield AgentEvent::Done { text: String::new(), history: Vec::new() };
                return;
            }
        };
        let api_mode = resolved.api_mode.clone();
        if api_mode == crate::models::ApiMode::Responses {
            // Responses branch keeps a chat-shaped history for persistence, translating to input each turn
            let model_id = resolved.model.clone();
            let client = Client::from_resolved(&resolved);
            let system = build_system_prompt().await;
            let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system})];
            let hist_slice = if history.len() > 20 { &history[history.len()-20..] } else { &history[..] };
            for v in hist_slice {
                let mut val = v.clone();
                if let Some(content) = val.get("content") { if let Some(s) = content.as_str() { if s.len() > 3000 { val["content"] = json!(format!("{}… [truncated]", &s[..3000])); } } }
                messages.push(val);
            }
            messages.push(json!({"role": "user", "content": &user_prompt}));
            let mut final_text = String::new();
            let mut tracker = PlanTracker::new(&user_prompt);
            for step in 0..max_steps {
                yield AgentEvent::Step { n: step + 1 };
                if step > 0 {
                    let context: String = messages.iter().rev().take(4).filter_map(|m| m.get("content").and_then(|c| c.as_str())).collect::<Vec<&str>>().join(" ");
                    if let Some(memory_note) = crate::memory::autorecall(&context) {
                        let recall_msg = json!({"role": "system", "content": memory_note});
                        if messages.len() > 1 && messages[1].get("role").and_then(|r| r.as_str()) == Some("system") && messages[1].get("content").and_then(|c| c.as_str()).map(|c| c.starts_with("Recalled memories")).unwrap_or(false) { messages[1] = recall_msg; } else { messages.insert(1, recall_msg); }
                    }
                }
                // Build instructions+input from current messages (system prompt already separate)
                // Focus context is appended to instructions each turn
                let focus = tracker.focus_context(step + 1);
                // Base instructions is system (build_system_prompt) plus any system messages in history; chat_messages_to_responses_input will merge them
                let (mut instructions, input) = llm::chat_messages_to_responses_input(&messages, &system);
                // Append focus to instructions (remove any previous focus by truncating? easiest: just append; next iteration will rebuild from scratch stripping old focus)
                // To avoid accumulating, we rebuild instructions each time from fresh system + messages that may already contain focus as system message. We inserted focus as system message below for chat compatibility, but for responses we keep it in instructions only.
                // We inserted focus as system message into messages last iteration via continue_prompt; need to ensure it maps correctly. For responses, continue_prompt system message will become part of instructions on next iteration via chat_messages_to_responses_input, which is good. But we also want focus each step. So we merge focus into instructions directly.
                // Remove any prior [FOCUS CONTEXT system message from messages to avoid duplication; responses instructions will include current focus anyway.
                // First, strip any existing focus system messages from messages before translation (we already have input computed, but instructions was computed with them). Instead recompute clean instructions by filtering focus marker.
                // Simpler: compute instructions without focus, then append current focus.
                // Filter focus from instructions by removing marker sections (if system messages contained focus, they were already joined). Since we appended focus only to instructions string last time via this path, not to messages, the next iteration's messages won't contain old focus. So just append current focus to instructions.
                instructions = format!("{}\n\n{}", instructions, focus);
                // For continuation prompts, the messages vector may contain a [CONTINUATION REQUIRED] system message inserted at end of last step (when no tool). That message is already in messages and will be translated: for responses, a system message becomes part of instructions, so it will appear as instructions addition too. That's okay.
                // Build tools for responses
                let tools = llm::responses_tool_definitions().await;
                let body = llm::build_responses_request_body(&model_id, &instructions, &input, &tools);
                let resp = match client.http.post(client.responses_url()).header("Authorization", format!("Bearer {}", client.api_key)).header("Content-Type", "application/json").json(&body).send().await {
                    Ok(r) => r,
                    Err(e) => { yield AgentEvent::Text { delta: format!("\n[LLM error: {}]", e) }; break; }
                };
                if !resp.status().is_success() {
                    let txt = resp.text().await.unwrap_or_default();
                    yield AgentEvent::Text { delta: format!("\n[LLM HTTP error: {}]", txt) };
                    break;
                }
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
                                                // also keep reverse for lookup if delta uses fc
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
                                        // also resolve via pending if fc key exists but we keyed by call
                                        let resolved_key = if tool_acc.contains_key(&key) { key.clone() } else if let Some(mapped) = pending_calls.get(&fc_id) { mapped.clone() } else { key.clone() };
                                        if !resolved_key.is_empty() {
                                            let entry = tool_acc.entry(resolved_key.clone()).or_insert_with(|| ToolAccum { id: call_id.clone(), name: String::new(), args: String::new() });
                                            if !name.is_empty() { entry.name = name; }
                                            if let Some(args) = item.arguments { if !args.is_empty() { entry.args = args; } }
                                            if !call_id.is_empty() { entry.id = call_id.clone(); }
                                        }
                                    }
                                },
                                llm::ResponsesEvent::FunctionCallArgsDelta { delta, item_id, call_id, .. } => {
                                    let raw = call_id.clone().or(item_id.clone()).unwrap_or_default();
                                    // Resolve fc -> call via pending_calls
                                    let resolved = if let Some(mapped) = pending_calls.get(&raw) { mapped.clone() } else { raw.clone() };
                                    let key = if tool_acc.contains_key(&resolved) { resolved.clone() } else if tool_acc.contains_key(&raw) { raw.clone() } else { resolved.clone() };
                                    if key.is_empty() {
                                        if let Some((k, _)) = tool_acc.iter().next().map(|(k,v)| (k.clone(), v)) {
                                            if let Some(entry) = tool_acc.get_mut(&k) { entry.args.push_str(&delta); }
                                        } else {
                                            // No entry yet — create placeholder; name will be filled by OutputItemAdded/Done
                                            let entry = tool_acc.entry("call_0".to_string()).or_insert_with(|| ToolAccum { id: "call_0".to_string(), name: String::new(), args: String::new() });
                                            entry.args.push_str(&delta);
                                        }
                                    } else {
                                        let entry = tool_acc.entry(key.clone()).or_insert_with(|| ToolAccum { id: key.clone(), name: String::new(), args: String::new() });
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
                                    }
                                }
                            },
                            llm::ResponsesEvent::FunctionCallArgsDelta { delta, item_id, call_id, .. } => {
                                let raw = call_id.clone().or(item_id.clone()).unwrap_or_default();
                                let target = if tool_acc.contains_key(&raw) { raw.clone() } else if let Some((k,_)) = tool_acc.iter().next().map(|(k,v)|(k.clone(),v)) { k.clone() } else { raw.clone() };
                                if target.is_empty() { if let Some((k,_))=tool_acc.iter().next().map(|(k,v)|(k.clone(),v)) { if let Some(e)=tool_acc.get_mut(&k){e.args.push_str(&delta);} } } else { let e=tool_acc.entry(target.clone()).or_insert_with(|| ToolAccum{id: target.clone(), name:String::new(), args:String::new()}); e.args.push_str(&delta); }
                            },
                            _ => {}
                        }
                    } else if let Ok(chunk) = serde_json::from_str::<llm::ChatChunk>(data) {
                        for ch in chunk.choices { if let Some(t)=ch.delta.content{accum_text.push_str(&t); yield AgentEvent::Text{delta:t}; } if let Some(tcs)=ch.delta.tool_calls { for tc in tcs { let key=tc.id.clone().unwrap_or_else(||format!("idx_{}",tc.index)); let e=tool_acc.entry(key.clone()).or_insert_with(||ToolAccum{id:tc.id.clone().unwrap_or(key),name:String::new(),args:String::new()}); if let Some(id)=tc.id{if !id.is_empty(){e.id=id;}} if let Some(f)=tc.function{if let Some(n)=f.name{if !n.is_empty(){e.name=n;}} if let Some(a)=f.arguments{e.args.push_str(&a);}} } } }
                    }
                }
                if !accum_text.is_empty() { final_text.push_str(&accum_text); yield AgentEvent::TextDone { text: accum_text.clone() }; tracker.record_text(&accum_text); }
                // If no tool calls, handle conversational / continuation logic same as chat
                if tool_acc.is_empty() {
                    if tracker.is_conversational_goal() {
                        if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
                        yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                        break;
                    }
                    tracker.nocall_streak += 1;
                    let complete = PlanTracker::looks_complete(&accum_text);
                    if complete || tracker.nocall_streak >= MAX_NOCALL_STREAK {
                        if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
                        yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                        break;
                    }
                    if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
                    let continue_msg = json!({"role": "system", "content": tracker.continue_prompt()});
                    messages.push(continue_msg);
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
                let futs: Vec<_> = ordered.iter().map(|(_, acc)| { let name=acc.name.clone(); let id=acc.id.clone(); let args_val: Value=serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone())); async move { let start=std::time::Instant::now(); let result=crate::tools::execute_tool(&name, args_val.clone()).await; let elapsed_ms=start.elapsed().as_millis() as u64; (id,name,result,args_val,elapsed_ms) }}).collect();
                let results = futures::future::join_all(futs).await;
                for (id, name, result, args_val, elapsed_ms) in results {
                    let display = result.find("<<IMAGE:").map_or_else(|| result.clone(), |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()));
                    yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                    tool_results.push((id,name,result,args_val));
                }
                let tool_calls_json: Vec<Value> = ordered.iter().map(|(_, acc)| json!({"id": acc.id, "type": "function", "function": {"name": acc.name, "arguments": acc.args}})).collect();
                messages.push(json!({"role": "assistant", "content": accum_text, "tool_calls": tool_calls_json}));
                for (id, _name, result, _) in tool_results {
                    let truncated = truncate_for_llm(&result);
                    let content: Value = if let Some(img_start) = result.find("<<IMAGE:") {
                        let meta_line = result[..img_start].trim_end();
                        let after_marker = &result[img_start+8..];
                        if let Some(colon_pos) = after_marker.find(':') { let mime=&after_marker[..colon_pos]; let b64=after_marker[colon_pos+1..].trim_end_matches(">>"); json!([{"type":"text","text":meta_line},{"type":"image_url","image_url":{"url":format!("data:{};base64,{}",mime,b64)}}]) } else { json!(truncated) }
                    } else { json!(truncated) };
                    messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
                }
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
        let hist_slice = if history.len() > 20 { &history[history.len()-20..] } else { &history[..] };
        for v in hist_slice {
            let mut val = v.clone();
            if let Some(content) = val.get("content") { if let Some(s) = content.as_str() { if s.len() > 3000 { val["content"] = json!(format!("{}… [truncated]", &s[..3000])); } } }
            messages.push(val);
        }
        messages.push(json!({"role": "user", "content": &user_prompt}));
        let mut final_text = String::new();
        let mut tracker = PlanTracker::new(&user_prompt);
        for step in 0..max_steps {
            yield AgentEvent::Step { n: step + 1 };
            if step > 0 {
                let context: String = messages.iter().rev().take(4).filter_map(|m| m.get("content").and_then(|c| c.as_str())).collect::<Vec<&str>>().join(" ");
                if let Some(memory_note) = crate::memory::autorecall(&context) {
                    let recall_msg = json!({"role": "system", "content": memory_note});
                    if messages.len() > 1 && messages[1].get("role").and_then(|r| r.as_str()) == Some("system") && messages[1].get("content").and_then(|c| c.as_str()).map(|c| c.starts_with("Recalled memories")).unwrap_or(false) { messages[1] = recall_msg; } else { messages.insert(1, recall_msg); }
                }
            }
            let focus_msg = json!({"role": "system", "content": tracker.focus_context(step + 1)});
            let marker = "[FOCUS CONTEXT";
            let mut removed_old = false;
            for i in (2..messages.len()).rev() {
                if messages[i].get("role").and_then(|r| r.as_str()) == Some("system") && messages[i].get("content").and_then(|c| c.as_str()).map(|c| c.starts_with(marker)).unwrap_or(false) { messages.remove(i); removed_old = true; break; }
            }
            let insert_at = if removed_old { 2 } else { 2 };
            if insert_at < messages.len() { messages.insert(insert_at, focus_msg); } else { messages.push(focus_msg); }
            let tools = llm::tool_definitions().await;
            let body = json!({"model": model_id, "messages": messages, "tools": tools, "tool_choice": "auto", "stream": true});
            let resp = match client.http.post(client.chat_url()).header("Authorization", format!("Bearer {}", client.api_key)).header("Content-Type", "application/json").json(&body).send().await {
                Ok(r) => r,
                Err(e) => { yield AgentEvent::Text { delta: format!("\n[LLM error: {}]", e) }; break; }
            };
            if !resp.status().is_success() {
                let txt = resp.text().await.unwrap_or_default();
                yield AgentEvent::Text { delta: format!("\n[LLM HTTP error: {}]", txt) };
                break;
            }
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
                    if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
                    yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                    break;
                }
                tracker.nocall_streak += 1;
                let complete = PlanTracker::looks_complete(&accum_text);
                if complete || tracker.nocall_streak >= MAX_NOCALL_STREAK {
                    yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                    break;
                }
                if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
                let continue_msg = json!({"role": "system", "content": tracker.continue_prompt()});
                messages.push(continue_msg);
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
                let display = result.find("<<IMAGE:").map_or_else(|| result.clone(), |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()));
                yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                tool_results.push((id, name, result, args_val));
            }
            let tool_calls_json: Vec<Value> = ordered.iter().map(|(_, acc)| json!({"id": acc.id, "type": "function", "function": {"name": acc.name, "arguments": acc.args}})).collect();
            messages.push(json!({"role": "assistant", "content": accum_text, "tool_calls": tool_calls_json}));
            for (id, _name, result, _) in tool_results {
                let truncated = truncate_for_llm(&result);
                let content: Value = if let Some(img_start) = result.find("<<IMAGE:") {
                    let meta_line = result[..img_start].trim_end();
                    let after_marker = &result[img_start + 8..];
                    if let Some(colon_pos) = after_marker.find(':') {
                        let mime = &after_marker[..colon_pos];
                        let b64 = after_marker[colon_pos + 1..].trim_end_matches(">>");
                        json!([{"type": "text", "text": meta_line},{"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime, b64)}}])
                    } else { json!(truncated) }
                } else { json!(truncated) };
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
            }
            let _ = usage;
        }
        yield AgentEvent::Done { text: final_text, history: messages.clone() };
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;
    const BASE_BUDGET: usize = 2500;
    const TOTAL_BUDGET: usize = 4000;

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
}
