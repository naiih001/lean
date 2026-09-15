use crate::core::modes::{is_ask_mode, is_plan_mode, set_mode, Mode};
use serde_json::Value;

pub(crate) fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "write" | "write_file" | "edit" | "edit_file" | "bash") || name.contains("__")
}

pub(crate) fn is_plan_exempt_write(name: &str, args: &Value) -> bool {
    if !is_plan_mode() {
        return false;
    }
    if name != "write" && name != "write_file" {
        return false;
    }
    if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
        return p.starts_with(".lean/plans")
            || p.starts_with("./.lean/plans")
            || p.contains("/.lean/plans");
    }
    false
}

pub(crate) fn is_permission_to_leave_plan(result: &str) -> bool {
    let lower = result.to_lowercase();
    lower.contains("leave plan and implement") || lower.contains("yes, leave plan")
}

pub(crate) fn is_stay_in_plan(result: &str) -> bool {
    let lower = result.to_lowercase();
    lower.contains("stay in plan")
}

pub(crate) fn is_readonly_bash(cmd: &str) -> bool {
    let lower = cmd.trim().to_lowercase();
    if lower.is_empty() {
        return true;
    }
    let stripped = lower
        .replace("2>/dev/null", "")
        .replace("2>&1", "")
        .replace("1>/dev/null", "")
        .replace(">/dev/null", "")
        .replace("2>&2", "");
    if stripped.contains('>') {
        return false;
    }
    let mutating = [
        " rm ", " rm", "rm ", "mv ", "cp ", "mkdir", "touch ", "chmod", "chown", "sed -i", "tee ",
        "rmdir", "unlink ", "shred ",
    ];
    for m in mutating {
        if lower.contains(m) {
            return false;
        }
    }
    if lower.starts_with("cargo build")
        || lower.starts_with("cargo test")
        || lower.starts_with("cargo run")
        || lower.starts_with("npm run")
        || lower.starts_with("npm install")
        || lower.starts_with("git commit")
        || lower.starts_with("git push")
        || lower.starts_with("git checkout")
        || lower.starts_with("git merge")
    {
        return false;
    }
    true
}

pub(crate) fn is_mcp_read(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("read")
        || lower.contains("list")
        || lower.contains("get")
        || lower.contains("search")
        || lower.contains("query")
        || lower.contains("fetch")
}

pub(crate) fn is_conversational_str(goal: &str) -> bool {
    let g = goal.trim().to_lowercase();
    let stripped = g
        .trim_matches(|c: char| {
            c == '!' || c == '.' || c == ',' || c == '?' || c == '\'' || c == '"'
        })
        .trim();
    let conversational_exact = [
        "hi",
        "hello",
        "hey",
        "hi there",
        "hello there",
        "hey there",
        "thanks",
        "thank you",
        "thanks!",
        "thank you!",
        "yo",
        "sup",
        "howdy",
        "hola",
        "how are you",
        "how are you?",
        "hey!",
        "hello!",
        "hi!",
    ];
    if conversational_exact.contains(&stripped) {
        return true;
    }
    if stripped.len() >= 30 {
        return false;
    }
    let has_task_verb = [
        "write",
        "create",
        "fix",
        "build",
        "edit",
        "read",
        "search",
        "make",
        "add",
        "update",
        "implement",
        "explain",
        "help with",
        "can you",
        "could you",
        "please",
        "run",
        "test",
        "refactor",
        "remove",
        "delete",
    ]
    .iter()
    .any(|v| stripped.contains(v));
    if has_task_verb {
        return false;
    }
    let greet_prefixes = ["hi ", "hello ", "hey ", "thanks ", "thank you "];
    if greet_prefixes.iter().any(|p| stripped.starts_with(p)) {
        return true;
    }
    if stripped.split_whitespace().count() <= 3 {
        if ["hi", "hello", "hey"].iter().any(|w| stripped.contains(w)) {
            return true;
        }
    }
    false
}

#[derive(Debug)]
pub struct PlanTracker {
    goal: String,
    steps_done: Vec<String>,
    last_tools: Vec<String>,
    pub(crate) nocall_streak: usize,
    requires_approval: bool,
    approved: bool,
}

pub(crate) const MAX_NOCALL_STREAK: usize = 3;

const PENDING_MARKERS: [&str; 7] = [
    "next step",
    "still need",
    "remaining",
    "todo",
    "then i",
    "i'll now",
    "let me",
];
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
    pub fn new(goal: &str) -> Self {
        let requires_approval = is_plan_mode() && !is_conversational_str(goal);
        Self {
            goal: goal.to_string(),
            steps_done: Vec::new(),
            last_tools: Vec::new(),
            nocall_streak: 0,
            requires_approval,
            approved: false,
        }
    }

    pub fn requires_approval(&self) -> bool {
        self.requires_approval
    }
    pub fn has_approval(&self) -> bool {
        self.approved
    }
    pub fn note_ask_result(&mut self, result: &str) {
        let lower = result.to_lowercase();
        if lower.contains("proceed as proposed")
            || lower.contains("\u{2713} proceed")
            || lower.contains("✓ proceed")
        {
            self.approved = true;
        }
        if is_permission_to_leave_plan(result) {
            set_mode(Mode::Norm);
            self.approved = true;
            self.requires_approval = false;
        } else if is_stay_in_plan(result) {
            self.requires_approval = is_plan_mode() && !is_conversational_str(&self.goal);
        }
    }

    pub fn looks_complete(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        if PENDING_MARKERS.iter().any(|m| lower.contains(m)) {
            return false;
        }
        COMPLETE_SIGNALS.iter().any(|s| lower.contains(s))
    }

    pub fn is_conversational_goal(&self) -> bool {
        is_conversational_str(&self.goal)
    }

    pub fn focus_context(&self, step: usize) -> String {
        if self.is_conversational_goal() {
            return format!("[Focus — conversational]\nGoal: \"{}\" — this is small talk. Reply warmly in 1-2 sentences and stop. No tools needed.\nStep: {}.", self.goal, step);
        }
        if is_ask_mode() {
            let mut out = String::from("[Focus — ASK read-only]\n");
            out.push_str(&format!("Goal: {}\n", self.goal));
            out.push_str("ASK is read-only: Allowed: read, read_skill, web_search, search_memory/recall_memory/list_memories, ask_user, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, 2>/dev/null, 2>&1, pipes), MCP reads (read/list/get/search/query/fetch). BLOCKED: write/edit/mutating bash (> file, rm/mv/cp/mkdir, cargo build/test/run, npm install, git commit/push) and MCP writes — reply with \"ASK is read-only — switch to Norm (Shift+Tab) or Plan to build.\" if asked to build. Do not over-verify; answer directly.\n");
            if !self.steps_done.is_empty() {
                out.push_str(&format!("Progress ({}):\n", self.steps_done.len()));
                for (i, s) in self.steps_done.iter().enumerate() {
                    out.push_str(&format!("  {}. {}\n", i + 1, s));
                }
            }
            if !self.last_tools.is_empty() {
                out.push_str(&format!("Recent tools: {}\n", self.last_tools.join(", ")));
            }
            out.push_str(&format!("Step {}.\n", step));
            return out;
        }
        if self.requires_approval && !self.approved {
            let mut out = String::from("[Focus — REAL-TASK GATING ACTIVE]\n");
            out.push_str(&format!("Goal: {}\n", self.goal));
            out.push_str("Phase: you are in Phases 1-4 (Discover → Clarify → Propose → Wait). MUTATING tools (write, edit, bash with > file, any MCP write) are BLOCKED until user selects \"\u{2713} Proceed as proposed\" via ask_user.\n");
            out.push_str("Allowed now: read (read-only), read_skill, web_search, search_memory/recall_memory/list_memories, ask_user, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, 2>/dev/null, 2>&1, pipes) and MCP reads — they auto-run without approval.\n");
            out.push_str("You MUST call ask_user now to clarify scope/approach. Cover goal, non-goals, files in scope, constraints, edge cases. Iterate until 100% sure. Final gating question MUST contain option exactly `\u{2713} Proceed as proposed`. Do NOT call mutating tools.\n");
            if !self.steps_done.is_empty() {
                out.push_str(&format!("Progress ({}):\n", self.steps_done.len()));
                for (i, s) in self.steps_done.iter().enumerate() {
                    out.push_str(&format!("  {}. {}\n", i + 1, s));
                }
            }
            if !self.last_tools.is_empty() {
                out.push_str(&format!("Recent tools: {}\n", self.last_tools.join(", ")));
            }
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
            for (i, s) in self.steps_done.iter().enumerate() {
                out.push_str(&format!("  {}. {}\n", i + 1, s));
            }
        } else {
            out.push_str("Progress: starting\n");
        }
        if !self.last_tools.is_empty() {
            out.push_str(&format!("Recent tools: {}\n", self.last_tools.join(", ")));
        }
        if self.nocall_streak > 0 {
            out.push_str(&format!("No tool call yet (attempt {}/{}): call a tool now, or if the work is done, summarize and end with \"All done.\"\n", self.nocall_streak, MAX_NOCALL_STREAK));
        }
        out.push_str(&format!("Step {}. Stay on track: take the next concrete step and don't repeat completed actions, don't re-read same file, don't re-verify. If the goal is met (and verified once if you mutated files), summarize and end with \"All done.\"\n", step));
        out
    }

    pub fn record_tools(&mut self, tool_names: &[String]) {
        self.nocall_streak = 0;
        self.last_tools = tool_names.to_vec();
        for name in tool_names {
            self.steps_done.push(format!("called {}", name));
        }
    }
    pub fn record_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let summary = if let Some(period) = trimmed.find('.') {
            if period < 200 {
                trimmed[..period + 1].to_string()
            } else {
                trimmed.chars().take(200).collect()
            }
        } else {
            trimmed.chars().take(200).collect()
        };
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
                let lower = summary.to_lowercase();
                if (lower.contains("thank you")
                    || lower.contains("thanks")
                    || lower.contains("you're welcome"))
                    && (prev.to_lowercase().contains("thank")
                        || prev.to_lowercase().contains("welcome"))
                {
                    return;
                }
            }
        }
        self.steps_done.push(summary);
    }
}
