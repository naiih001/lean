use crate::llm::{self, Client};
use crate::skills;
use futures::Stream;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const REGULAR_SYSTEM_PROMPT: &str = "You are lean, a coding assistant in the terminal. Be direct, concise, and verify your work.\n\n\
## When to act\n\
- Greeting or small talk with no request (\"hi\", \"thanks\", \"how are you\") → reply warmly in 1-2 sentences and stop. No tools, no follow-up.\n\
- Otherwise → task mode (doing).\n\n\
## Task mode (regular — doing)\n\
1. Understand: read the relevant files before editing.\n\
2. Act: use tools (read_file, edit_file, write_file, bash, web_search). Make the smallest change that solves the problem.\n\
3. Verify: run the build, tests, or lint that covers the change; don't assume success.\n\
4. Summarize: state what changed and end with \"All done.\"\n\
Continue while steps remain. Stop when the goal is met and verified. If a tool fails, read the error and adjust; don't repeat a call that already succeeded.\n\n\
## Tools\n\
- Read before edit; use a unique oldText for precise edits.\n\
- If you need a file, call read_file now instead of saying you will.\n\
- Only respond as the assistant. Never write a user \"thanks\" or \"you're welcome\" on the user's behalf.\n\n\
## Asking the user\n\
- If a request is genuinely ambiguous (unclear target, scope, or preference) and you can't discover the answer from the repo, call ask_user with concrete options instead of guessing.\n\
- Don't ask when you can find the answer yourself. For simple, low-risk tasks, bias toward doing — call the tool and verify.\n\n\
## Skills and memory\n\
- Skills are markdown workflows listed below. If one matches the task, call read_skill and follow it.\n\
- Search memory only when prior context helps (multi-turn, user preference, project fact). Call remember when you learn something worth keeping.\n";

pub const PLAN_SYSTEM_PROMPT: &str = "You are lean, a coding assistant in the terminal. Be direct, concise, and verify your work. You are in PLAN MODE — you plan, you do not implement (except the plan file).\n\n\
## When to act\n\
- Greeting or small talk with no request (\"hi\", \"thanks\", \"how are you\") → reply warmly in 1-2 sentences and stop. No tools, no follow-up.\n\
- Otherwise → Real-task planning mode. This includes write tasks, and also read-only investigations, explanations, and searches when the user wants a plan.\n\n\
## Task classification (do this silently before Phase 1)\n\
Classify the request:\n\
- `write` — needs file writes/edits, mutating bash, or MCP tools.\n\
- `explain` — answer about code/content without mutation.\n\
- `search` — needs web lookup.\n\
If unsure, treat as `write`.\n\n\
## Plan mode — 5-phase gate (MANDATORY)\n\
You MUST NOT call write_file, edit_file, bash, or any MCP tool (name contains `__`) on project files until you have completed Phases 1-4 and received explicit user approval via ask_user. Read-only tools (read_file, read_skill, web_search, search_memory, etc.) are always allowed. In plan mode, the ONLY write allowed before approval is `write_file` to `.hermes/plans/` for the deliverable plan. All other mutations are BLOCKED until Phase 4 approval.\n\n\
Phase 1 — DISCOVER (read-only): read relevant files, search memory/skills, gather context. No mutations.\n\
Phase 2 — CLARIFY: call ask_user with concrete options until scope is 100% clear. For each ambiguity present 2-3 options with pros/cons. Cover: goal, non-goals, files/modules in scope, UX/constraints, edge cases. Keep asking — do not assume.\n\n\
Phase 3 — PROPOSE: write a concrete plan markdown to `.hermes/plans/YYYY-MM-DD_HHMMSS-<slug>.md` (see plan skill for template: goal, context, approach, steps, files, tests, risks). Then summarize Shared Understanding (scope + chosen approach + files + verification) and ask a final ask_user question that MUST contain an option exactly labeled `✓ Proceed as proposed` (and `Needs changes` / Other).\n\n\
Phase 4 — WAIT: Do NOT mutate project files. If user selects `✓ Proceed as proposed` → approved (plan is done; user will run implementation separately or ask you to implement). If Other/Needs changes → loop back to Phase 2.\n\n\
Phase 5 — (only if user explicitly asks to implement after plan approval): execute the approved plan, verify, summarize and end with `All done.` Otherwise, end after plan is written and approved.\n\n\
Continue while steps remain. If a tool fails, read the error and adjust; don't repeat a call that already succeeded. Do not act on inferred intent before Phase 4 approval.\n\n\
## Tools\n\
- Read before edit; use a unique oldText for precise edits.\n\
- If you need a file, call read_file now instead of saying you will.\n\
- Only respond as the assistant. Never write a user \"thanks\" or \"you're welcome\" on the user's behalf.\n\n\
## Asking the user\n\
- In plan mode, you MUST use ask_user in Phases 2-3 — to confirm scope, constraints, and approach and to get explicit `✓ Proceed as proposed` approval. Iterate until no assumptions remain. Ask until you are 100% sure.\n\n\
## Skills and memory\n\
- Skills are markdown workflows listed below. If one matches the task, call read_skill and follow it — especially `plan` in this mode.\n\
- Search memory only when prior context helps (multi-turn, user preference, project fact). Call remember when you learn something worth keeping.\n";

pub const SYSTEM_PROMPT: &str = REGULAR_SYSTEM_PROMPT;

use std::sync::atomic::{AtomicBool, Ordering};
static PLAN_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_plan_mode(v: bool) { PLAN_MODE.store(v, Ordering::Relaxed); }
pub fn is_plan_mode() -> bool { PLAN_MODE.load(Ordering::Relaxed) }
pub fn toggle_plan_mode() -> bool {
    let cur = is_plan_mode();
    set_plan_mode(!cur);
    !cur
}

fn current_system_prompt() -> &'static str {
    if is_plan_mode() { PLAN_SYSTEM_PROMPT } else { REGULAR_SYSTEM_PROMPT }
}

const TOTAL_BUDGET: usize = 4000;
const SKILL_MAX_COUNT: usize = 8;
const SKILL_LINE_MAX: usize = 120;

/// Truncate to at most `max` characters, never splitting a UTF-8 boundary.
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut t: String = s.chars().take(max - 1).collect();
    t.push('…');
    t
}

/// Truncate to at most `max` characters without appending a marker.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Truncate so the result stays within `max_bytes`, honoring UTF-8 boundaries.
fn truncate_to_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let budget = max_bytes.saturating_sub(3);
    let mut out = String::new();
    for ch in s.chars() {
        if out.len() + ch.len_utf8() > budget {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// Render the skill catalog as `- name: description` lines, capped at `max_lines`.
fn render_skill_catalog(raw_catalog: &str, max_lines: usize) -> String {
    if raw_catalog.starts_with("No skills") {
        return truncate_str(raw_catalog, 300);
    }
    let lines: Vec<&str> = raw_catalog.lines().collect();
    let take = max_lines.min(lines.len());
    let mut out: Vec<String> = Vec::with_capacity(take + 1);
    for line in lines.iter().take(take) {
        // line is "- name: desc (path: ...)" — keep "- name: desc", drop the path
        let short = match line.find(" (path:") {
            Some(idx) => &line[..idx],
            None => line,
        };
        out.push(truncate_str(short, SKILL_LINE_MAX));
    }
    if lines.len() > take {
        out.push(format!("... +{} more (use read_skill to see)", lines.len() - take));
    }
    out.join("\n")
}

fn skills_section(catalog: &str) -> String {
    format!(
        "\n\n## Available Skills\n{}\n\nIf a skill matches the task, call read_skill to load its full guide.",
        catalog
    )
}

fn confinement_section() -> Option<String> {
    if crate::dir_guard::is_disabled() {
        return None;
    }
    let cwd = crate::dir_guard::project_root().display().to_string();
    Some(truncate_str(
        &format!("\n\n## Confinement\nYou are confined to CWD: `{}`. Paths outside need approval.", cwd),
        300,
    ))
}

fn mcp_section() -> String {
    let mcp_snap = crate::mcp::snapshot();
    if mcp_snap.is_empty() {
        return "\n\n## MCP\nNo MCP servers configured.".to_string();
    }
    let mut s = String::from("\n\n## MCP Servers (server__tool, needs approval)\n");
    for srv in &mcp_snap {
        s.push_str(&format!("- {} [{}] ({} tools)", srv.name, srv.status.as_str(), srv.tools.len()));
        if !srv.tools.is_empty() {
            let names: Vec<String> = srv.tools.iter().take(5).map(|t| t.name.clone()).collect();
            s.push_str(&format!(": {}", names.join(", ")));
            if srv.tools.len() > 5 {
                s.push_str(&format!(" +{} more", srv.tools.len() - 5));
            }
        }
        if let Some(err) = &srv.error_detail {
            s.push_str(&format!(" — {}", truncate_str(err, 80)));
        }
        s.push('\n');
    }
    truncate_str(&s, 400)
}

/// Assemble the full system prompt within `TOTAL_BUDGET`.
///
/// Priority order: the base prompt (regular or plan) is never truncated, the confinement
/// guard is kept when enabled, the skill catalog is shrunk line by line, and the
/// MCP section is dropped first when the budget is exceeded.
pub async fn build_system_prompt() -> String {
    let base = current_system_prompt();
    let raw_catalog = skills::get_skill_catalog().await;
    let confinement = confinement_section();

    let assemble = |catalog: &str| {
        let mut prompt = format!("{}{}", base, skills_section(catalog));
        if let Some(note) = &confinement {
            prompt.push_str(note);
        }
        prompt
    };

    let prompt = assemble(&render_skill_catalog(&raw_catalog, SKILL_MAX_COUNT));
    if prompt.len() <= TOTAL_BUDGET {
        let with_mcp = format!("{}{}", prompt, mcp_section());
        if with_mcp.len() <= TOTAL_BUDGET {
            return with_mcp;
        }
    }

    // Over budget: shrink the skill catalog a line at a time, dropping MCP first.
    for lines in (1..SKILL_MAX_COUNT).rev() {
        let candidate = assemble(&render_skill_catalog(&raw_catalog, lines));
        if candidate.len() <= TOTAL_BUDGET {
            return candidate;
        }
    }

    // Last resort: base prompt plus confinement, bounded by bytes.
    let mut minimal = String::from(base);
    if let Some(note) = &confinement {
        minimal.push_str(note);
    }
    truncate_to_bytes(&minimal, TOTAL_BUDGET)
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

pub(crate) fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file" | "bash") || name.contains("__")
}

pub(crate) fn is_plan_exempt_write(name: &str, args: &Value) -> bool {
    if !is_plan_mode() { return false; }
    if name != "write_file" { return false; }
    if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
        return p.starts_with(".hermes/plans") || p.starts_with("./.hermes/plans") || p.contains("/.hermes/plans");
    }
    false
}

fn is_conversational_str(goal: &str) -> bool {
    let g = goal.trim().to_lowercase();
    let stripped = g.trim_matches(|c: char| c == '!' || c == '.' || c == ',' || c == '?' || c == '\'' || c == '"').trim();
    let conversational_exact = ["hi","hello","hey","hi there","hello there","hey there","thanks","thank you","thanks!","thank you!","yo","sup","howdy","hola","how are you","how are you?","hey!","hello!","hi!"];
    if conversational_exact.contains(&stripped) { return true; }
    if stripped.len() >= 30 { return false; }
    let has_task_verb = ["write","create","fix","build","edit","read","search","make","add","update","implement","explain","help with","can you","could you","please","run","test","refactor","remove","delete"].iter().any(|v| stripped.contains(v));
    if has_task_verb { return false; }
    let greet_prefixes = ["hi ","hello ","hey ","thanks ","thank you "];
    if greet_prefixes.iter().any(|p| stripped.starts_with(p)) { return true; }
    if stripped.split_whitespace().count() <= 3 {
        if ["hi","hello","hey"].iter().any(|w| stripped.contains(w)) { return true; }
    }
    false
}

struct PlanTracker {
    goal: String,
    steps_done: Vec<String>,
    last_tools: Vec<String>,
    nocall_streak: usize,
    requires_approval: bool,
    approved: bool,
}

const MAX_NOCALL_STREAK: usize = 3;

const PENDING_MARKERS: [&str; 7] = ["next step", "still need", "remaining", "todo", "then i", "i'll now", "let me"];
const COMPLETE_SIGNALS: [&str; 10] = [
    "here's what i did",
    "here is what i did",
    "summary:",
    "in summary",
    "that completes",
    "all done",
    "task complete",
    "i've finished",
    "i have finished",
    "done.",
];

impl PlanTracker {
    fn new(goal: &str) -> Self {
        // Plan mode = strict 5-phase gate; Regular mode = no gating, bias to doing.
        let requires_approval = is_plan_mode() && !is_conversational_str(goal);
        Self { goal: goal.to_string(), steps_done: Vec::new(), last_tools: Vec::new(), nocall_streak: 0, requires_approval, approved: false }
    }

    fn requires_approval(&self) -> bool { self.requires_approval }
    fn has_approval(&self) -> bool { self.approved }
    fn set_approved(&mut self, v: bool) { self.approved = v; }
    fn note_ask_result(&mut self, result: &str) {
        let lower = result.to_lowercase();
        if lower.contains("proceed as proposed") || lower.contains("\u{2713} proceed") || lower.contains("✓ proceed") {
            self.approved = true;
        }
        // If user said Needs changes / Other with feedback, reset to not approved so we loop
        // We keep approved=true only on explicit proceed; any other ask_user result keeps blocked until proceed is seen.
    }

    /// True only when the reply carries a terminal signal and nothing is still pending.
    fn looks_complete(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        if PENDING_MARKERS.iter().any(|m| lower.contains(m)) {
            return false;
        }
        COMPLETE_SIGNALS.iter().any(|s| lower.contains(s))
    }

    fn is_conversational_goal(&self) -> bool {
        is_conversational_str(&self.goal)
    }

    fn focus_context(&self, step: usize) -> String {
        if self.is_conversational_goal() {
            return format!("[Focus — conversational]\nGoal: \"{}\" — this is small talk. Reply warmly in 1-2 sentences and stop. No tools needed.\nStep: {}.", self.goal, step);
        }
        // Gating: Phases 1-4 — mutating tools blocked until Proceed
        if self.requires_approval && !self.approved {
            let mut out = String::from("[Focus — REAL-TASK GATING ACTIVE]\n");
            out.push_str(&format!("Goal: {}\n", self.goal));
            out.push_str("Phase: you are in Phases 1-4 (Discover → Clarify → Propose → Wait). MUTATING tools (write_file, edit_file, bash, any MCP __) are BLOCKED until user selects \"\u{2713} Proceed as proposed\" via ask_user.\n");
            out.push_str("Allowed now: read_file (read-only), read_skill, web_search, search_memory/recall_memory/list_memories, ask_user.\n");
            out.push_str("You MUST call ask_user now to clarify scope/approach. Cover goal, non-goals, files in scope, constraints, edge cases. Iterate until 100% sure. Final gating question MUST contain option exactly `\u{2713} Proceed as proposed`. Do NOT call mutating tools.\n");
            if !self.steps_done.is_empty() {
                out.push_str(&format!("Progress ({}):\n", self.steps_done.len()));
                for (i, s) in self.steps_done.iter().enumerate() { out.push_str(&format!("  {}. {}\n", i+1, s)); }
            }
            if !self.last_tools.is_empty() { out.push_str(&format!("Recent tools: {}\n", self.last_tools.join(", "))); }
            if self.nocall_streak > 0 {
                out.push_str(&format!("No tool call yet (attempt {}/{}): call ask_user now to clarify, or if already clarified, ask the final Proceed question.\n", self.nocall_streak, MAX_NOCALL_STREAK));
            }
            out.push_str(&format!("Step {}.\n", step));
            return out;
        }
        let mut out = String::from("[Focus]\n");
        out.push_str(&format!("Goal: {}\n", self.goal));
        if self.requires_approval && self.approved {
            out.push_str("Gating: APPROVED — you have received `\u{2713} Proceed as proposed`. You may now use all tools in Phase 5 (Act).\n");
        }
        if !self.steps_done.is_empty() {
            out.push_str(&format!("Progress ({}):\n", self.steps_done.len()));
            for (i, s) in self.steps_done.iter().enumerate() { out.push_str(&format!("  {}. {}\n", i+1, s)); }
        } else { out.push_str("Progress: starting\n"); }
        if !self.last_tools.is_empty() { out.push_str(&format!("Recent tools: {}\n", self.last_tools.join(", "))); }
        if self.nocall_streak > 0 {
            out.push_str(&format!("No tool call yet (attempt {}/{}): call a tool now, or if the work is done, summarize and end with \"All done.\"\n", self.nocall_streak, MAX_NOCALL_STREAK));
        }
        out.push_str(&format!("Step {}. Stay on track: take the next concrete step and don't repeat completed actions. If the goal is met and verified, summarize and end with \"All done.\"\n", step));
        out
    }

    fn record_tools(&mut self, tool_names: &[String]) {
        self.nocall_streak = 0;
        self.last_tools = tool_names.to_vec();
        for name in tool_names { self.steps_done.push(format!("called {}", name)); }
    }
    fn record_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let summary = if let Some(period) = trimmed.find('.') {
            if period < 200 { trimmed[..period+1].to_string() } else { trimmed.chars().take(200).collect() }
        } else { trimmed.chars().take(200).collect() };
        // Dedup: Jaccard >0.75 vs last 2 entries, like consolidate_memory
        if !self.steps_done.is_empty() {
            let new_tokens: std::collections::HashSet<String> = summary
                .to_lowercase()
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            for prev in self.steps_done.iter().rev().take(2) {
                let prev_tokens: std::collections::HashSet<String> = prev
                    .to_lowercase()
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                let inter = new_tokens.intersection(&prev_tokens).count() as f32;
                let union = new_tokens.union(&prev_tokens).count() as f32;
                if union > 0.0 && inter / union > 0.75 {
                    return;
                }
                // Also hard block thank-you echo
                let lower = summary.to_lowercase();
                if (lower.contains("thank you") || lower.contains("thanks") || lower.contains("you're welcome"))
                    && (prev.to_lowercase().contains("thank") || prev.to_lowercase().contains("welcome"))
                {
                    return;
                }
            }
        }
        self.steps_done.push(summary);
    }
}

#[derive(PartialEq)]
enum ContextKind {
    Focus,
    Continue,
}

fn classify_context_message(value: &Value) -> Option<ContextKind> {
    if value.get("role").and_then(|r| r.as_str()) != Some("system") {
        return None;
    }
    let content = value.get("content").and_then(|c| c.as_str())?;
    if content.starts_with("[Focus") || content.starts_with("[FOCUS") {
        Some(ContextKind::Focus)
    } else if content.starts_with("[Continue") || content.starts_with("[CONTINUATION") {
        Some(ContextKind::Continue)
    } else {
        None
    }
}

/// Drop injected focus/continue context so it cannot accumulate across steps.
/// Resumed sessions may still carry older `[Continue]` messages, so both kinds
/// are removed and the caller re-injects a fresh focus message each turn.
fn prune_context_messages(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .filter(|m| classify_context_message(m).is_none())
        .cloned()
        .collect()
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
                if let Some(content) = val.get("content") { if let Some(s) = content.as_str() { if s.chars().count() > 3000 { val["content"] = json!(format!("{}… [truncated]", truncate_chars(s, 3000))); } } }
                messages.push(val);
            }
            messages.push(json!({"role": "user", "content": &user_prompt}));
            let mut final_text = String::new();
            let mut tracker = PlanTracker::new(&user_prompt);
            for step in 0..max_steps {
                yield AgentEvent::Step { n: step + 1 };
                if step > 0 && !tracker.is_conversational_goal() {
                    let context: String = messages.iter().rev().take(4).filter_map(|m| m.get("content").and_then(|c| c.as_str())).collect::<Vec<&str>>().join(" ");
                    if context.trim().len() > 20 {
                        if let Some(memory_note) = crate::memory::autorecall(&context) {
                            // Budget guard: skip if would push instructions near limit
                            if memory_note.len() + 500 < 3800 {
                                let recall_msg = json!({"role": "system", "content": memory_note});
                                if messages.len() > 1 && messages[1].get("role").and_then(|r| r.as_str()) == Some("system") && messages[1].get("content").and_then(|c| c.as_str()).map(|c| c.starts_with("Recalled memories")).unwrap_or(false) { messages[1] = recall_msg; } else { messages.insert(1, recall_msg); }
                            }
                        }
                    }
                }
                let focus = tracker.focus_context(step + 1);
                // Re-inject focus each turn from a history with stale injected context removed,
                // so instructions stay bounded instead of accumulating across steps.
                let pruned = prune_context_messages(&messages);
                let (mut instructions, input) = llm::chat_messages_to_responses_input(&pruned, &system);
                instructions = format!("{}\n\n{}", instructions, focus);
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
                    let complete = tracker.looks_complete(&accum_text);
                    if complete || tracker.nocall_streak >= MAX_NOCALL_STREAK {
                        if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
                        yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                        break;
                    }
                    if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
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
                let futs: Vec<_> = ordered.iter().map(|(_, acc)| { let name=acc.name.clone(); let id=acc.id.clone(); let args_val: Value=serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone())); let blocked = gate_active && is_mutating_tool(&name) && !is_plan_exempt_write(&name, &args_val); async move { let start=std::time::Instant::now(); let result = if blocked { format!("[GATING BLOCKED — plan mode] Mutating tool '{}' is blocked until you complete Phases 1-4 and get explicit user approval via ask_user with '\\u{{2713}} Proceed as proposed'. Call ask_user now to clarify scope/approach. In plan mode only .hermes/plans writes are allowed before approval; all other mutations blocked. (Toggle plan mode off with /plan if you want regular doing mode.)", name) } else { crate::tools::execute_tool(&name, args_val.clone()).await }; let elapsed_ms=start.elapsed().as_millis() as u64; (id,name,result,args_val,elapsed_ms) }}).collect();
                let results = futures::future::join_all(futs).await;
                for (id, name, result, args_val, elapsed_ms) in results {
                    let display = result.find("<<IMAGE:").map_or_else(|| result.clone(), |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()));
                    yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                    // If this was ask_user, check for Proceed approval
                    if name == "ask_user" {
                        tracker.note_ask_result(&result);
                    }
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
            if let Some(content) = val.get("content") { if let Some(s) = content.as_str() { if s.chars().count() > 3000 { val["content"] = json!(format!("{}… [truncated]", truncate_chars(s, 3000))); } } }
            messages.push(val);
        }
        messages.push(json!({"role": "user", "content": &user_prompt}));
        let mut final_text = String::new();
        let mut tracker = PlanTracker::new(&user_prompt);
        for step in 0..max_steps {
            yield AgentEvent::Step { n: step + 1 };
            if step > 0 && !tracker.is_conversational_goal() {
                let context: String = messages.iter().rev().take(4).filter_map(|m| m.get("content").and_then(|c| c.as_str())).collect::<Vec<&str>>().join(" ");
                if context.trim().len() > 20 {
                    if let Some(memory_note) = crate::memory::autorecall(&context) {
                        if memory_note.len() + 500 < 3800 {
                            let recall_msg = json!({"role": "system", "content": memory_note});
                            if messages.len() > 1 && messages[1].get("role").and_then(|r| r.as_str()) == Some("system") && messages[1].get("content").and_then(|c| c.as_str()).map(|c| c.starts_with("Recalled memories")).unwrap_or(false) { messages[1] = recall_msg; } else { messages.insert(1, recall_msg); }
                        }
                    }
                }
            }
            // Drop any stale focus/continue context, then re-inject a fresh focus
            // message right after the system prompt so it never accumulates.
            messages = prune_context_messages(&messages);
            let focus_msg = json!({"role": "system", "content": tracker.focus_context(step + 1)});
            if messages.len() > 1 { messages.insert(1, focus_msg); } else { messages.push(focus_msg); }
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
                let complete = tracker.looks_complete(&accum_text);
                if complete || tracker.nocall_streak >= MAX_NOCALL_STREAK {
                    yield AgentEvent::Done { text: final_text.clone(), history: messages.clone() };
                    break;
                }
                if !accum_text.is_empty() { messages.push(json!({"role": "assistant", "content": accum_text})); }
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
            let futs: Vec<_> = ordered.iter().map(|(_, acc)| {
                let name = acc.name.clone();
                let id = acc.id.clone();
                let args_val: Value = serde_json::from_str(&acc.args).unwrap_or(Value::String(acc.args.clone()));
                let blocked = gate_active && is_mutating_tool(&name) && !is_plan_exempt_write(&name, &args_val);
                async move {
                    let start = std::time::Instant::now();
                    let result = if blocked { format!("[GATING BLOCKED — plan mode] Mutating tool '{}' is blocked until you complete Phases 1-4 and get explicit user approval via ask_user with '\\u{{2713}} Proceed as proposed'. Call ask_user now to clarify scope/approach. In plan mode only .hermes/plans writes are allowed before approval; all other mutations blocked. (Toggle plan mode off with /plan if you want regular doing mode.)", name) } else { crate::tools::execute_tool(&name, args_val.clone()).await };
                    let elapsed_ms = start.elapsed().as_millis() as u64;
                    (id, name, result, args_val, elapsed_ms)
                }
            }).collect();
            let results = futures::future::join_all(futs).await;
            for (id, name, result, args_val, elapsed_ms) in results {
                let display = result.find("<<IMAGE:").map_or_else(|| result.clone(), |pos| format!("{}[image data omitted for display]", result[..pos].trim_end()));
                yield AgentEvent::ToolResult { name: name.clone(), result: display, id: id.clone(), elapsed_ms };
                if name == "ask_user" {
                    tracker.note_ask_result(&result);
                }
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
    const BASE_BUDGET: usize = 3600;
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

    #[tokio::test]
    async fn built_prompt_keeps_base_prompt_intact() {
        let built = build_system_prompt().await;
        assert!(built.starts_with(SYSTEM_PROMPT), "base prompt was altered or truncated");
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
        for goal in ["hi", "hello", "hey", "thanks", "thank you", "how are you?", "Hi!", "Hey there"] {
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
        assert!(pruned[1]["content"].as_str().unwrap().starts_with("Recalled memories"));
        assert_eq!(pruned[2]["content"], "hi");
    }
}
