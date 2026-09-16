use crate::services::skills;

pub const REGULAR_SYSTEM_PROMPT: &str = "You are lean, a coding assistant in the terminal. Be direct and concise.\n\n\
## When to act\n\
- Greeting or small talk with no request (\"hi\", \"thanks\", \"how are you\") → reply warmly in 1-2 sentences and stop. No tools, no follow-up.\n\
- Otherwise → task mode (doing).\n\n\
## Task mode (regular — issue-solving)\n\
1. Understand: identify likely files/commands, read relevant files before editing, use search (grep/find) when target not obvious.\n\
2. Diagnose: form a concrete hypothesis from file contents, errors, or test output. Do not claim complete before a diagnostic read/search.\n\
3. Act: use tools (read, edit, write, bash, grep, find, ls, web_search, web_fetch). Make the smallest useful change; prefer precise edits with unique oldText. Allow repeated reads/edits when prior attempt failed, context changed, or verification exposed a new issue.\n\
4. Verify (if you mutated files): run the smallest relevant check (e.g. cargo check, focused test) and handle the result. If blocked, state the concrete blocker and lower-confidence evidence.\n\
5. Summarize: state what changed, verification result, and residual risk if any. End with \"All done.\" only when evidence supports completion. Do NOT produce intermediate summaries — until verification has passed, only call tools; do not say \"All done\", \"here's what I did\" or rephrase the same summary.\n\
- Read relevant files before editing. Prefer small changes.\n\
- Do not repeat successful actions without reason. Do repeat inspection or edits when a prior attempt failed, context changed, or verification exposed a new issue.\n\
- Stop only after evidence: for mutation tasks, you need either (a) mutation + passing verification, (b) mutation + blocked verification with explanation, (c) no mutation needed with file/tool evidence, or (d) a stated blocker (missing input/permission/dependency/repeated failure). Never complete from phrase alone (\"All done.\", \"here's what I did\") after a failed edit or failed verification.\n\
- On failure, treat tool output as new evidence and continue: edit oldText not found → re-read/search then retry; malformed args → retry with valid args; blocked tool → use allowed alternative or explain blocker; compiler/test failure → diagnose and edit; truncated output → narrow command or read file directly. Change at least one of file target, search query, command, edit range, or hypothesis; avoid identical retries.\n\n\
## Tools\n\
- Read before edit; use a unique oldText for precise edits. If you need a file, call read now instead of saying you will.\n\
- Only respond as the assistant. Never write a user \"thanks\" or \"you're welcome\" on the user's behalf.\n\n\
## Asking the user\n\
- If a request is genuinely ambiguous (unclear target, scope, or preference) and you can't discover the answer from the repo, call ask_user with concrete options instead of guessing.\n\
- Don't ask when you can find the answer yourself. For simple, low-risk tasks, bias toward doing — call the tool and stop.\n\n\
## Skills and memory\n\
- Skills are markdown workflows listed below. BEFORE acting, scan Available Skills — if one matches the task, you SHOULD call read_skill and follow it (strongly recommended, not mandatory; skip only if clearly irrelevant). Prefer skill guidance over improvising.\n\
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
You MUST NOT call write, edit, bash (mutating), or any MCP write tool on project files until you have completed Phases 1-4, received `✓ Proceed as proposed`, AND received explicit permission to leave PLAN mode via ask_user. Read-only tools (read, read_skill, web_search, search_memory, etc., plus read-only bash like ls/cat/grep/find and MCP reads) are always allowed. In plan mode, the ONLY write allowed before leaving is `write` to `.lean/plans/` for the deliverable plan. All other mutations are BLOCKED until you leave PLAN.\n\n\
Phase 1 — DISCOVER (read-only): read relevant files, search memory/skills, gather context. No mutations.\n\
Phase 2 — CLARIFY: call ask_user with concrete options until scope is 100% clear. For each ambiguity present 2-3 options with pros/cons. Cover: goal, non-goals, files/modules in scope, UX/constraints, edge cases. Keep asking — do not assume.\n\n\
Phase 3 — PROPOSE: write a concrete plan markdown to `.lean/plans/YYYY-MM-DD_HHMMSS-<slug>.md` (see plan skill for template: goal, context, approach, steps, files, tests, risks). Then summarize Shared Understanding (scope + chosen approach + files + verification) and ask a final ask_user question that MUST contain an option exactly labeled `✓ Proceed as proposed` (and `Needs changes` / Other).\n\n\
Phase 4 — WAIT: Do NOT mutate project files. If user selects `✓ Proceed as proposed` → plan approved. If Other/Needs changes → loop back to Phase 2.

Phase 4b — LEAVE PERMISSION (required before any building): When you are done planning and want to start building, you MUST call ask_user with header `Leave PLAN?`, question `Leave PLAN mode and start building?` and options exactly `[\"✓ Yes, leave PLAN and implement\", \"Stay in PLAN\"]` (do not add extra options; Other row will appear but you should not rely on it). If user selects `✓ Yes, leave PLAN and implement` → you will be switched to NORM automatically and may proceed to Phase 5. If `Stay in PLAN` → remain in PLAN, do not mutate.\n\n\
Phase 5 — (only after Leave permission granted): switch to NORM and execute the approved plan, verify once with ONE minimal check, summarize and end with `All done.` Otherwise, end after plan is written and approved.\n\n\
Continue while steps remain but don't loop or over-verify. If a tool fails, read the error and adjust; don't repeat a call that already succeeded. Do not act on inferred intent before Phase 4 approval.\n\n\
## Tools\n\
- Read before edit; use a unique oldText for precise edits.\n\
- If you need a file, call read now instead of saying you will.\n\
- Only respond as the assistant. Never write a user \"thanks\" or \"you're welcome\" on the user's behalf.\n\n\
## Asking the user\n\
- In plan mode, you MUST use ask_user in Phases 2-3 — to confirm scope, constraints, and approach and to get explicit `✓ Proceed as proposed` approval. Iterate until no assumptions remain. Ask until you are 100% sure.\n\n\
## Skills and memory\n\
- Skills are markdown workflows listed below. BEFORE acting, scan Available Skills — if one matches the task, you SHOULD call read_skill and follow it (strongly recommended, not mandatory; skip only if clearly irrelevant) — especially `plan` in this mode. Prefer skill guidance over improvising.\n\
- Search memory only when prior context helps (multi-turn, user preference, project fact). Call remember when you learn something worth keeping.\n";

pub const ASK_SYSTEM_PROMPT: &str = "You are lean, a coding assistant in the terminal. You are in ASK MODE — read-only, answer without mutating.\n\n\
## When to act\n\
- Greeting or small talk with no request (\"hi\", \"thanks\", \"how are you\") → reply warmly in 1-2 sentences and stop. No tools, no follow-up.\n\
- Otherwise → read-only task mode.\n\n\
## Task mode (ASK — read-only)\n\
1. Understand: read relevant files, search memory/skills, gather context. No mutations.\n\
2. Answer: use only read-only tools: read, read_skill, web_search, search_memory/recall_memory/list_memories, ask_user, readonly bash (ls/cat/grep/find/rg/git log|status|diff|show, plus stderr redirects 2>/dev/null and 2>&1 and pipes), and MCP reads (tools with read/list/get/search/query/fetch). Make no file writes or edits.\n\
3. Summarize: state what you found and how to proceed. If the user wants you to build/edit, tell them: \"ASK is read-only — switch to Norm (Shift+Tab) or Plan to build.\"\n\
Don't loop or re-read the same file. Continue while steps remain but stop when answered — no verification needed in ASK. If a tool fails, read the error and adjust; don't repeat a succeeded call. Never call write, edit, mutating bash (rm/mv/cp/mkdir/touch/chmod/chown/sed -i/tee/rmdir/unlink/shred, cargo build/test/run, npm run/install/publish, git commit/push/checkout/merge, or any > file / >> file redirection), or MCP writes — they are BLOCKED.\n\n\
## Tools\n\
- Read before edit would be in Norm; in Ask just read and search.\n\
- If you need a file, call read now instead of saying you will.\n\
- Only respond as the assistant. Never write a user \"thanks\" or \"you're welcome\" on the user's behalf.\n\n\
## Asking the user\n\
- If genuinely ambiguous and you can't discover the answer, call ask_user with concrete options. Otherwise answer directly; don't over-ask.\n\n\
## Skills and memory\n\
- Skills are markdown workflows listed below. BEFORE acting, scan Available Skills — if one matches the task, you SHOULD call read_skill and follow it (strongly recommended, not mandatory; skip only if clearly irrelevant). Prefer skill guidance over improvising.\n\
- Search memory only when prior context helps. Call remember when you learn something worth keeping.\n";

pub const ASK_READONLY_DENY_MSG: &str =
    "ASK is read-only — switch to Norm (Shift+Tab) or Plan to build.";

pub const SYSTEM_PROMPT: &str = REGULAR_SYSTEM_PROMPT;

pub(crate) fn current_system_prompt() -> &'static str {
    if crate::core::modes::is_plan_mode() {
        PLAN_SYSTEM_PROMPT
    } else if crate::core::modes::is_ask_mode() {
        ASK_SYSTEM_PROMPT
    } else {
        REGULAR_SYSTEM_PROMPT
    }
}

pub(crate) const TOTAL_BUDGET: usize = 12000;
const SKILL_LINE_MAX: usize = 200;

/// Truncate to at most `max` characters, never splitting a UTF-8 boundary.
pub(crate) fn truncate_str(s: &str, max: usize) -> String {
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
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Truncate so the result stays within `max_bytes`, honoring UTF-8 boundaries.
pub(crate) fn truncate_to_bytes(s: &str, max_bytes: usize) -> String {
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

pub(crate) const MAX_TOOL_OUTPUT_FOR_LLM: usize = 2000;

pub(crate) fn truncate_for_llm(s: &str) -> String {
    if s.len() <= MAX_TOOL_OUTPUT_FOR_LLM {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX_TOOL_OUTPUT_FOR_LLM).collect();
    let remaining = s.chars().count() - MAX_TOOL_OUTPUT_FOR_LLM;
    format!(
        "{}… [truncated {} chars for LLM, full shown in TUI]",
        truncated, remaining
    )
}

/// Render the skill catalog as `- name: description` lines.
fn render_skill_catalog(raw_catalog: &str) -> String {
    if raw_catalog.starts_with("No skills") {
        return truncate_str(raw_catalog, 300);
    }
    raw_catalog
        .lines()
        .map(|line| {
            let short = match line.find(" (path:") {
                Some(idx) => &line[..idx],
                None => line,
            };
            truncate_str(short, SKILL_LINE_MAX)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn skills_section(catalog: &str) -> String {
    format!(
        "\n\n## Available Skills\n{}\n\nBefore acting, scan this list — if a skill matches the task, you SHOULD call read_skill and follow it (strongly recommended, not mandatory; skip only if clearly irrelevant). Prefer skill guidance over improvising.",
        catalog
    )
}

fn confinement_section() -> Option<String> {
    if crate::guards::dir::is_disabled() {
        return None;
    }
    let cwd = crate::guards::dir::project_root().display().to_string();
    Some(truncate_str(
        &format!(
            "\n\n## Confinement\nYou are confined to CWD: `{}`. Paths outside need approval.",
            cwd
        ),
        300,
    ))
}

async fn agents_section() -> Option<String> {
    let catalog = crate::services::agents::get_agent_catalog().await;
    if catalog.starts_with("No agents") {
        return None;
    }
    let lines: Vec<&str> = catalog.lines().collect();
    let take = 6.min(lines.len());
    let mut out = String::from("\n\n## Available Agents (subagents)\n");
    out.push_str("You SHOULD delegate via `subagent` when it saves context or parallelizes work (requires unique `name` label, e.g. subagent(agent=\"scout\", task=\"...\", name=\"research-auth\") — label is shown first in popup, auto-suffixed if duplicate). Use scout for recon, researcher for web, worker for general tasks. Prefer delegation for exploration/recon over doing it inline. Users can add agents via agents/<name>/AGENTS.md\n");
    for line in lines.iter().take(take) {
        out.push_str(line);
        out.push_str("\n");
    }
    if lines.len() > take {
        out.push_str(&format!(
            "... +{} more (use read_agent or subagents_list)\n",
            lines.len() - take
        ));
    }
    Some(truncate_str(&out, 800))
}

fn context_section() -> Option<String> {
    crate::core::context::load_context_section().map(|s| format!("\n\n{}", s))
}

fn mcp_section() -> String {
    let mcp_snap = crate::integrations::mcp::snapshot();
    if mcp_snap.is_empty() {
        return "\n\n## MCP\nNo MCP servers configured.".to_string();
    }
    let mut s = String::from("\n\n## MCP Servers (server__tool, needs approval)\n");
    for srv in &mcp_snap {
        s.push_str(&format!(
            "- {} [{}] ({} tools)",
            srv.name,
            srv.status.as_str(),
            srv.tools.len()
        ));
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
pub async fn build_system_prompt() -> String {
    let base = current_system_prompt();
    let raw_catalog = skills::get_skill_catalog().await;
    let confinement = confinement_section();
    let context = context_section();

    let assemble = |catalog: &str, ctx: Option<&str>, agents: Option<&str>| {
        let mut prompt = String::from(base);
        if let Some(c) = ctx {
            prompt.push_str(c);
        }
        prompt.push_str(&skills_section(catalog));
        if let Some(a) = agents {
            prompt.push_str(a);
        }
        if let Some(note) = &confinement {
            prompt.push_str(note);
        }
        prompt
    };

    let agents = agents_section().await;
    // hard guarantee: full catalog (name + description) always emitted; bodies stay lazy via read_skill
    let full_catalog = render_skill_catalog(&raw_catalog);
    let prompt = assemble(&full_catalog, context.as_deref(), agents.as_deref());
    if prompt.len() <= TOTAL_BUDGET {
        let with_mcp = format!("{}{}", prompt, mcp_section());
        if with_mcp.len() <= TOTAL_BUDGET {
            return with_mcp;
        }
    }

    // Overflow: shrink non-skill sections first, never drop skills.
    if let Some(ctx) = &context {
        let mut minimal_ctx = truncate_to_bytes(
            ctx,
            TOTAL_BUDGET.saturating_sub(
                base.len()
                    + skills_section(&full_catalog).len()
                    + confinement.as_ref().map(|s| s.len()).unwrap_or(0)
                    + agents.as_deref().map(|s| s.len()).unwrap_or(0)
                    + 500,
            ),
        );
        if minimal_ctx.len() < ctx.len() {
            minimal_ctx.push_str("\n… [context truncated for budget]");
        }
        let candidate = assemble(&full_catalog, Some(&minimal_ctx), agents.as_deref());
        if candidate.len() <= TOTAL_BUDGET {
            return candidate;
        }
        // Drop agents next, keep full catalog.
        let candidate_no_agents = assemble(&full_catalog, Some(&minimal_ctx), None);
        if candidate_no_agents.len() <= TOTAL_BUDGET {
            return candidate_no_agents;
        }
        let candidate_no_ctx = assemble(&full_catalog, None, agents.as_deref());
        if candidate_no_ctx.len() <= TOTAL_BUDGET {
            return candidate_no_ctx;
        }
        let mut skills_only = String::from(base);
        skills_only.push_str(&skills_section(&full_catalog));
        if let Some(note) = &confinement {
            skills_only.push_str(note);
        }
        if skills_only.len() <= TOTAL_BUDGET {
            return skills_only;
        }
    } else {
        // No context: try without agents before hard guarantee.
        let candidate_no_agents = assemble(&full_catalog, None, None);
        if candidate_no_agents.len() <= TOTAL_BUDGET {
            return candidate_no_agents;
        }
    }

    // Hard guarantee fallback: return full catalog even if over TOTAL_BUDGET.
    let mut minimal = String::from(base);
    minimal.push_str(&skills_section(&full_catalog));
    if let Some(note) = &confinement {
        minimal.push_str(note);
    }
    if minimal.len() <= TOTAL_BUDGET {
        return minimal;
    }
    minimal
}
