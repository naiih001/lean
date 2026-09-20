use crate::core::modes::{is_ask_mode, is_plan_mode, set_mode, Mode};
use crate::core::prompts::truncate_str;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

/// Approved plan path pending injection into the next build turn (take-once).
/// Set on leave-PLAN approval, consumed by the next focus_context call so the
/// build turn starts with the plan file in context (opencode-style handoff).
static LAST_APPROVED_PLAN: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn approved_plan_cell() -> &'static Mutex<Option<String>> {
    LAST_APPROVED_PLAN.get_or_init(|| Mutex::new(None))
}

/// Take the pending approved plan path, if any.
pub fn take_approved_plan() -> Option<String> {
    approved_plan_cell()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

pub(crate) fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "write" | "write_file" | "edit" | "edit_file" | "bash") || name.contains("__")
}

pub(crate) fn is_plan_exempt_write(name: &str, args: &Value) -> bool {
    if !is_plan_mode() {
        return false;
    }
    if !matches!(name, "write" | "write_file" | "edit" | "edit_file") {
        return false;
    }
    args.get("path")
        .and_then(|v| v.as_str())
        .and_then(plan_file_path)
        .is_some()
}

fn normalize_rel_path(p: &str) -> String {
    // Lexical normalization (no disk access): unify separators, drop "." and
    // empty segments, resolve ".." — so `plans/../../etc` can't smuggle out.
    let mut parts: Vec<&str> = Vec::new();
    let unified = p.replace('\\', "/");
    for comp in unified.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    parts.join("/")
}

/// Normalized plans-dir-relative path when `p` points inside `.lean/plans`
/// (relative or absolute). Used for the plan-mode write exemption and for
/// recording the approved plan path for the plan→build handoff.
pub(crate) fn plan_file_path(p: &str) -> Option<String> {
    let n = normalize_rel_path(p);
    if n.starts_with(".lean/plans/") {
        return Some(n);
    }
    // Absolute path containing the dir, e.g. /home/u/proj/.lean/plans/x.md.
    if let Some(idx) = n.find("/.lean/plans/") {
        return Some(n[idx + 1..].to_string());
    }
    None
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
    // Interpreters and mutating commands matched by invocation position
    // (command start or after a shell separator) — not substring, so
    // `grep python` stays read-only while `python x.py` does not.
    // Fail direction is safe: unknown → approval/block, never silent allow.
    const INVOKES_DENY: &[&str] = &[
        "python",
        "perl",
        "ruby",
        "node",
        "php",
        "bash",
        "sh ",
        "curl",
        "wget",
        "ssh",
        "scp",
        "git apply",
        "patch",
        "truncate",
        "ln ",
        "ln -s",
        "install ",
        "docker",
        "kubectl",
        "xargs",
        "mkfifo",
        "dd ",
    ];
    for prog in INVOKES_DENY {
        if invokes_command(&stripped, prog) {
            return false;
        }
    }
    if invokes_command(&stripped, "find")
        && (stripped.contains("-exec") || stripped.contains("-delete"))
    {
        return false;
    }
    // Device nodes (e.g. /dev/tcp for network, /dev/sda) are never readonly.
    if stripped.contains("/dev/") {
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

/// True when `prog` is invoked as a command: at the string start or right
/// after a shell separator (`;`, `&`, `|`, `(`, backtick, newline).
fn invokes_command(stripped: &str, prog: &str) -> bool {
    let norm = stripped
        .replace("&&", ";")
        .replace("||", ";")
        .replace('&', ";")
        .replace('|', ";")
        .replace('(', ";")
        .replace('`', ";")
        .replace('\n', ";");
    norm.split(';')
        .any(|seg| seg.trim_start().starts_with(prog))
}

pub(crate) fn is_mcp_read(name: &str) -> bool {
    // Match the tool part (after `server__`) against read verbs with a
    // separator-or-end boundary: `list_files` and `list` are reads, but
    // `getAndUpdate` is not (fail direction is safe — approval instead).
    let tool = name.rsplit("__").next().unwrap_or(name);
    let lower = tool.to_lowercase();
    const VERBS: &[&str] = &[
        "read", "list", "get", "search", "query", "fetch", "describe", "show",
    ];
    VERBS.iter().any(|v| {
        lower == *v
            || lower.starts_with(&format!("{}_", v))
            || lower.starts_with(&format!("{}-", v))
    })
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestMode {
    Conversational,
    ReadOnly,
    IssueSolving,
    Planning,
}

fn classify_request_mode(goal: &str) -> RequestMode {
    if is_plan_mode() {
        return RequestMode::Planning;
    }
    if is_ask_mode() {
        return RequestMode::ReadOnly;
    }
    if is_conversational_str(goal) {
        return RequestMode::Conversational;
    }
    let lower = goal.to_lowercase();
    let readonly_markers = [
        "explain",
        "inspect",
        "show",
        "describe",
        "what",
        "why",
        "how",
        "list",
        "search",
        "find",
        "tell me",
        "summarize",
        "overview",
        "read ",
        "read README",
    ];
    if readonly_markers.iter().any(|v| lower.contains(v)) {
        return RequestMode::ReadOnly;
    }
    let issue_verbs = [
        "fix",
        "add",
        "update",
        "remove",
        "refactor",
        "make",
        "create",
        "implement",
        "edit",
        "write",
        "delete",
        "build",
        "change",
        "patch",
        "resolve",
        "repair",
        "modify",
        "adjust",
        "correct",
        "handle",
        "improve",
        "test",
        "run",
    ];
    if issue_verbs.iter().any(|v| lower.contains(v)) {
        return RequestMode::IssueSolving;
    }
    // Default to issue-solving for non-trivial requests that imply work
    if lower.split_whitespace().count() > 3 {
        return RequestMode::IssueSolving;
    }
    RequestMode::ReadOnly
}

#[derive(Debug)]
pub struct PlanTracker {
    goal: String,
    steps_done: Vec<String>,
    last_tools: Vec<String>,
    pub(crate) nocall_streak: usize,
    requires_approval: bool,
    approved: bool,
    /// Plan file written during Phase 3 (for the plan→build handoff).
    plan_path: Option<String>,
    // Evidence-driven state (behaviour plan)
    pub(crate) mode: RequestMode,
    pub(crate) inspected: bool,
    pub(crate) mutation_attempted: bool,
    pub(crate) mutation_succeeded: bool,
    pub(crate) verification_attempted: bool,
    pub(crate) verification_passed: bool,
    pub(crate) verification_failed: bool,
    pub(crate) verification_blocked: bool,
    pub(crate) tool_failed: bool,
    pub(crate) has_blocker: bool,
    pub(crate) blocker_reason: Option<String>,
    pub(crate) last_failure: Option<String>,
    pub(crate) failed_signatures: Vec<String>,
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
        let mode = classify_request_mode(goal);
        Self {
            goal: goal.to_string(),
            steps_done: Vec::new(),
            last_tools: Vec::new(),
            nocall_streak: 0,
            requires_approval,
            approved: false,
            plan_path: None,
            mode,
            inspected: false,
            mutation_attempted: false,
            mutation_succeeded: false,
            verification_attempted: false,
            verification_passed: false,
            verification_failed: false,
            verification_blocked: false,
            tool_failed: false,
            has_blocker: false,
            blocker_reason: None,
            last_failure: None,
            failed_signatures: Vec::new(),
        }
    }

    pub fn request_mode(&self) -> &RequestMode {
        &self.mode
    }

    fn is_verification_command(cmd: &str) -> bool {
        let lower = cmd.to_lowercase();
        // Cargo / JS / generic project checks — broadened to avoid All-done loops on non-cargo projects
        lower.contains("cargo check")
            || lower.contains("cargo test")
            || lower.contains("cargo clippy")
            || lower.contains("cargo fmt")
            || lower.contains("cargo build")
            || lower.contains("npm test")
            || lower.contains("npm run")
            || lower.contains("pnpm")
            || lower.contains("yarn")
            || lower.contains("make test")
            || lower.contains("make check")
            || lower.contains("pytest")
            || lower.contains("go test")
            || lower.contains("svelte-check")
            || lower.contains("svelte-kit")
            || lower.contains("check")
            || lower.contains("lint")
            || lower.contains("build")
            || lower.contains("test")
    }

    fn classify_tool_result(result: &str) -> bool {
        let lower = result.to_lowercase();
        lower.contains("error:")
            || lower.contains("[error")
            || lower.contains("failed")
            || lower.contains("failure")
            || lower.contains("blocked")
            || lower.contains("not found")
            || lower.contains("old text not found")
            || lower.contains("unknown tool")
            || lower.contains("malformed")
    }

    pub fn note_tool_result(&mut self, name: &str, args: &Value, result: &str) {
        let is_failure = Self::classify_tool_result(result)
            || result.contains("[ASK BLOCKED]")
            || result.contains("[GATING BLOCKED")
            || result.contains("[dir-guard BLOCKED")
            || result.contains("[bash-guard BLOCKED");
        let sig = format!("{}:{}", name, args.to_string());
        if is_failure {
            self.tool_failed = true;
            self.last_failure = Some(result.chars().take(300).collect());
            if !self.failed_signatures.contains(&sig) {
                self.failed_signatures.push(sig);
                if self.failed_signatures.len() > 10 {
                    self.failed_signatures.remove(0);
                }
            }
            // Detect blocker conditions
            if result.contains("BLOCKED")
                || result.contains("approval")
                || result.contains("permission")
            {
                self.has_blocker = true;
                self.blocker_reason = Some(result.chars().take(300).collect());
            }
            if name == "edit" || name == "edit_file" {
                if result.to_lowercase().contains("old text not found") {
                    self.last_failure = Some("edit oldText not found".to_string());
                }
            }
        } else {
            // Success path — update evidence flags
            if matches!(
                name,
                "read"
                    | "read_file"
                    | "grep"
                    | "find"
                    | "ls"
                    | "web_search"
                    | "web_fetch"
                    | "read_skill"
                    | "read_agent"
            ) || name.contains("__") && is_mcp_read(name)
            {
                self.inspected = true;
            }
            if is_mutating_tool(name) && name != "bash" {
                self.mutation_attempted = true;
                self.mutation_succeeded = true;
                self.inspected = true;
            }
            if name == "bash" {
                if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                    if Self::is_verification_command(cmd) {
                        self.verification_attempted = true;
                        // Heuristic: failure already handled above; if not failure, treat as passed
                        self.verification_passed = true;
                        self.verification_failed = false;
                    } else if !is_readonly_bash(cmd) {
                        self.mutation_attempted = true;
                        self.mutation_succeeded = true;
                    } else {
                        self.inspected = true;
                    }
                }
            }
            if name == "write" || name == "write_file" {
                self.mutation_attempted = true;
                self.mutation_succeeded = true;
            }
            // Record plan files for the plan→build handoff (any mode — the
            // path only matters if a leave-PLAN approval follows).
            if matches!(name, "write" | "write_file" | "edit" | "edit_file") {
                if let Some(p) = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .and_then(plan_file_path)
                {
                    self.plan_path = Some(p);
                }
            }
        }
        // Verification failure/blocked override
        if Self::is_verification_command(args.get("command").and_then(|v| v.as_str()).unwrap_or(""))
        {
            if is_failure {
                self.verification_attempted = true;
                self.verification_failed = true;
                self.verification_passed = false;
                if result.contains("BLOCKED") {
                    self.verification_blocked = true;
                    self.has_blocker = true;
                }
            }
        }
    }

    pub fn can_complete(&self) -> bool {
        match self.mode {
            RequestMode::Conversational => true,
            RequestMode::ReadOnly => self.inspected || self.steps_done.len() >= 1,
            RequestMode::Planning => self.approved || self.inspected,
            RequestMode::IssueSolving => {
                if self.has_blocker {
                    return true;
                }
                if !self.mutation_attempted {
                    // No mutation needed — must have inspected evidence
                    return self.inspected;
                }
                // Mutation happened — need verification or explicit blocked reason
                if self.mutation_succeeded && self.verification_passed {
                    return true;
                }
                if self.mutation_succeeded && self.verification_blocked {
                    return true;
                }
                false
            }
        }
    }

    pub fn completion_blocker_hint(&self) -> Option<String> {
        if self.mode != RequestMode::IssueSolving {
            return None;
        }
        if !self.inspected && !self.mutation_attempted {
            return Some(
                "No inspection yet — read relevant files or search before editing.".to_string(),
            );
        }
        if self.mutation_succeeded && !self.verification_attempted {
            return Some("Mutation succeeded but verification not yet run — run the smallest relevant check (e.g. cargo check) or explain blocker.".to_string());
        }
        if self.verification_failed {
            return Some(
                "Verification failed — read the error, diagnose, and continue editing.".to_string(),
            );
        }
        if self.tool_failed {
            if let Some(last) = &self.last_failure {
                if last.contains("oldText not found") {
                    return Some("Edit failed (oldText not found) — re-read the file and retry with current content.".to_string());
                }
                return Some(format!(
                    "Last tool failed — address the error before summarizing: {}",
                    truncate_str(last, 200)
                ));
            }
            return Some(
                "A tool failed — retry with corrected args or an allowed alternative.".to_string(),
            );
        }
        None
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
            // Opencode-style handoff: the next build turn starts with the
            // approved plan path in context.
            if let Some(p) = self.plan_path.clone() {
                *approved_plan_cell()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(p);
            }
        } else if is_stay_in_plan(result) {
            self.requires_approval = is_plan_mode() && !is_conversational_str(&self.goal);
        }
    }

    pub fn looks_complete(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        if PENDING_MARKERS.iter().any(|m| lower.contains(m)) {
            return false;
        }
        let has_signal = COMPLETE_SIGNALS.iter().any(|s| lower.contains(s));
        if !has_signal {
            return false;
        }
        // Phrase alone is not enough for issue-solving — require evidence state
        if self.mode == RequestMode::IssueSolving && !self.can_complete() {
            return false;
        }
        true
    }

    /// State-based completion check without phrase — used by agent loop.
    pub fn is_state_complete(&self) -> bool {
        self.can_complete()
    }

    pub fn is_conversational_goal(&self) -> bool {
        is_conversational_str(&self.goal)
    }

    pub fn focus_context(&self, step: usize) -> String {
        if self.mode == RequestMode::Conversational || self.is_conversational_goal() {
            return format!("[Focus — conversational]\nGoal: \"{}\" — this is small talk. Reply warmly in 1-2 sentences and stop. No tools needed.\nStep: {}.", self.goal, step);
        }
        if is_ask_mode() {
            let mut out = String::from("[Focus — ASK read-only]\n");
            out.push_str(&format!("Goal: {}\n", self.goal));
            out.push_str("ASK is read-only: Allowed: read, read_skill, web_search, ask_user, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, 2>/dev/null, 2>&1, pipes), MCP reads (read/list/get/search/query/fetch). BLOCKED: write/edit/mutating bash (> file, rm/mv/cp/mkdir, cargo build/test/run, npm install, git commit/push) and MCP writes — reply with \"ASK is read-only — switch to Norm (Tab) or Plan to build.\" if asked to build. Do not over-verify; answer directly.\n");
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
            out.push_str("Allowed now: read (read-only), read_skill, web_search, ask_user, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, 2>/dev/null, 2>&1, pipes) and MCP reads — they auto-run without approval.\n");
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
        // Plan→build handoff: approved plan path injected once, on the first
        // focus call after leave-PLAN approval.
        if let Some(p) = take_approved_plan() {
            out.push_str(&format!("Approved plan: {} — build it now. Keep to the approved scope; verify once with one minimal check, then summarize.\n", p));
        }
        out.push_str(&format!(
            "Mode: {:?} | inspected={} mutation={}/{:?} verified={}/{:?} failed={}\n",
            self.mode,
            self.inspected,
            self.mutation_attempted,
            if self.mutation_succeeded { "ok" } else { "no" },
            self.verification_attempted,
            if self.verification_passed {
                "pass"
            } else if self.verification_failed {
                "fail"
            } else {
                "-"
            },
            self.tool_failed
        ));
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
        // Evidence-driven guidance
        if let Some(hint) = self.completion_blocker_hint() {
            out.push_str(&format!("Evidence gate: {}\n", hint));
        }
        if self.mode == RequestMode::IssueSolving
            && self.mutation_succeeded
            && !self.verification_attempted
            && !self.has_blocker
        {
            out.push_str("Required: run the smallest relevant verification (e.g. cargo check) before completing.\n");
        }
        if self.tool_failed {
            if let Some(sig) = self.failed_signatures.last() {
                out.push_str(&format!("Last failure signature: {} — change file target, search query, command, or edit range before retry; do not repeat identical call.\n", truncate_str(sig, 180)));
            }
        }
        if self.nocall_streak > 0 {
            if self.mode == RequestMode::IssueSolving && !self.inspected {
                out.push_str(&format!("No tool call yet (attempt {}/{}): inspect repo now — read relevant files or grep/find before editing.\n", self.nocall_streak, MAX_NOCALL_STREAK));
            } else if self.mode == RequestMode::IssueSolving
                && self.mutation_succeeded
                && !self.verification_attempted
            {
                out.push_str(&format!("No tool call yet (attempt {}/{}): verification required — run cargo check or focused test now.\n", self.nocall_streak, MAX_NOCALL_STREAK));
            } else {
                out.push_str(&format!("No tool call yet (attempt {}/{}): call a tool now, or if evidence shows completion, summarize and end with \"All done.\"\n", self.nocall_streak, MAX_NOCALL_STREAK));
            }
        }
        if self.can_complete() {
            out.push_str(&format!("Step {} — evidence gate PASSED; you may summarize now (state what changed, verification result, residual risk) and end with \"All done.\"\n", step));
        } else {
            out.push_str(&format!("Step {} — continue: inspect → diagnose → act → verify. Do not repeat successful actions without reason; do repeat after failure with new evidence.\n", step));
        }
        out
    }

    pub fn record_tools(&mut self, tool_names: &[String]) {
        self.nocall_streak = 0;
        self.last_tools = tool_names.to_vec();
        for name in tool_names {
            self.steps_done.push(format!("called {}", name));
            // Keep evidence flags in sync even before note_tool_result is called
            if matches!(
                name.as_str(),
                "read"
                    | "read_file"
                    | "grep"
                    | "find"
                    | "ls"
                    | "web_search"
                    | "web_fetch"
                    | "read_skill"
                    | "read_agent"
            ) || (name.contains("__") && is_mcp_read(name))
            {
                self.inspected = true;
            }
            if is_mutating_tool(name) && name.as_str() != "bash" {
                self.mutation_attempted = true;
                self.mutation_succeeded = true;
                self.inspected = true;
            }
            if name.as_str() == "write" || name.as_str() == "write_file" {
                self.mutation_attempted = true;
                self.mutation_succeeded = true;
            }
        }
    }
    pub fn record_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        // Don't record premature summaries as progress — they pollute the next focus
        // and cause the model to reply to its own intermediate thoughts.
        // Only record tool-related progress; free-form summaries are kept only if
        // they signal completion and evidence gate is passed.
        let lower = trimmed.to_lowercase();
        let is_summary_like = lower.contains("all done")
            || lower.contains("here's what i did")
            || lower.contains("here is what i did")
            || lower.contains("in summary")
            || lower.contains("that completes");
        if is_summary_like && self.mode == RequestMode::IssueSolving && !self.can_complete() {
            return;
        }
        // For issue-solving tasks, ignore intermediate free-form text that is not
        // a tool call and not yet evidence-complete. Keep only concise first sentence
        // and deduplicate aggressively to prevent over-replies.
        if self.mode == RequestMode::IssueSolving && !self.can_complete() {
            // If text looks like a rephrased summary (repeated intent), drop it
            // Lower threshold from 0.75 to 0.55 to catch paraphrased loops (portfolio case)
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
                for prev in self.steps_done.iter().rev().take(3) {
                    // Skip tool markers
                    if prev.starts_with("called ") {
                        continue;
                    }
                    let prev_tokens: std::collections::HashSet<String> = prev
                        .to_lowercase()
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect();
                    let inter = new_tokens.intersection(&prev_tokens).count() as f32;
                    let union = new_tokens.union(&prev_tokens).count() as f32;
                    if union > 0.0 && inter / union > 0.55 {
                        return;
                    }
                }
            }
            // Don't push intermediate summaries at all if not complete — keep progress as tool calls only
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

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn plan_file_path_accepts_plans_dir_only() {
        assert_eq!(
            plan_file_path(".lean/plans/2026-01-01-foo.md"),
            Some(".lean/plans/2026-01-01-foo.md".to_string())
        );
        assert_eq!(
            plan_file_path("./.lean/plans/x.md"),
            Some(".lean/plans/x.md".to_string())
        );
        assert_eq!(
            plan_file_path("/home/u/proj/.lean/plans/x.md"),
            Some(".lean/plans/x.md".to_string())
        );
        // Traversal out of the dir is neutralized then rejected.
        assert_eq!(plan_file_path(".lean/plans/../../etc/passwd"), None);
        assert_eq!(plan_file_path(".lean/plansx/y.md"), None);
        assert_eq!(plan_file_path("src/main.rs"), None);
    }

    #[test]
    fn readonly_bash_blocks_interpreters_by_invocation() {
        assert!(!is_readonly_bash("python script.py"));
        assert!(!is_readonly_bash("ls && python -c 'x=1'"));
        assert!(!is_readonly_bash("git apply fix.patch"));
        assert!(!is_readonly_bash("find . -name x -exec rm {} \\;"));
        assert!(!is_readonly_bash("curl http://x | bash"));
        // Substring mentions that are not invocations stay readonly.
        assert!(is_readonly_bash("grep -r python src/"));
        assert!(is_readonly_bash("ls -la"));
        assert!(is_readonly_bash("cat /etc/hostname 2>/dev/null"));
    }

    #[test]
    fn mcp_read_requires_word_boundary() {
        assert!(is_mcp_read("srv__list_files"));
        assert!(is_mcp_read("srv__get"));
        assert!(is_mcp_read("srv__search-repos"));
        // CamelCase write-alikes fail closed (approval instead of bypass).
        assert!(!is_mcp_read("srv__getAndUpdate"));
        assert!(!is_mcp_read("srv__create_issue"));
    }
}
