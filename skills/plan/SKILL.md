---
name: plan
description: Structured 5-phase planning before building — discover, clarify, propose, wait, implement. Use when task needs file writes, mutating bash, or exploration before mutation.
---

# Plan Skill — Lean 5-Phase Gate

Use this when the user wants a plan, or before any write/mutating work in PLAN mode. You are the source of truth for `.lean/plans/` deliverables.

## When to Use

- User says `/plan`, is in PLAN mode, or asks for a plan/design/proposal before building.
- Any task that will need `write_file`, `edit_file`, mutating `bash` (`cargo build`, `npm install`, `git commit`, `rm/mv/cp/mkdir`, `> file`, `sed -i`), or MCP write tools.
- Read-only tasks (`read_file`, `web_search`, `read_skill`, readonly `bash`, MCP reads) do **not** need a full plan — answer directly.

## The 5 Phases (MANDATORY in PLAN)

**Phase 1 — DISCOVER (read-only):** `read_file`, `read_skill`, `web_search`, `search_memory`, readonly `bash` (`ls/cat/grep/find/rg/git log|status|diff|show`, `2>/dev/null`, pipes), MCP reads. No mutations. `write_file` only to `.lean/plans/**` is allowed.

**Phase 2 — CLARIFY:** `ask_user` until 100% clear. For each ambiguity present 2-3 options with pros/cons. Cover: goal, non-goals, files/modules in scope, UX/constraints, edge cases.

**Phase 3 — PROPOSE:** Write concrete markdown to `.lean/plans/YYYY-MM-DD_HHMMSS-<slug>.md` with: goal, context, approach, steps, files, tests, risks. Then summarize Shared Understanding (scope + approach + files + verification) and ask final `ask_user` with an option exactly labeled `✓ Proceed as proposed` (plus `Needs changes` / Other).

**Phase 4 — WAIT:** Do not mutate. If `✓ Proceed as proposed` → approved. If Other/Needs changes → loop to Phase 2.

**Phase 4b — LEAVE PERMISSION:** To start building, call `ask_user` with header `Leave PLAN?`, question `Leave PLAN mode and start building?`, options exactly `["✓ Yes, leave PLAN and implement", "Stay in PLAN"]`. On `✓ Yes` you are switched to NORM and may proceed to Phase 5. On `Stay` remain gated.

**Phase 5 — BUILD** (only after Leave permission): Switch to NORM, execute approved plan, verify (`cargo check/test`), summarize and end `All done.`

## Allowed vs Blocked in Phases 1-4

- **Allowed auto-run:** `read_file`, `read_skill`, `web_search`, `search_memory`/`recall_memory`/`list_memories`, `ask_user`, readonly `bash`, MCP reads (`*read`, `*list`, `*get`, `*search`, `*query`, `*fetch`).
- **Blocked until Proceed:** `write_file` (except `.lean/plans/**`), `edit_file`, mutating `bash` (contains `rm/mv/cp/mkdir/touch/chmod/chown/sed -i/tee/rmdir/unlink/shred`, `cargo build/test/run`, `npm run/install/publish`, `git commit/push/checkout`, `> file`/`>> file`), MCP writes.

`ASK` mode is permanently read-only — same allowed set, same `> file` block, with deny message `ASK is read-only — switch to Norm (Shift+Tab) or Plan to build.` It never enters Phase 4/4b.

## Template

```markdown
# Plan: <short goal>

## Goal
One sentence.

## Context
Repo facts, files in scope, constraints.

## Approach
Chosen approach + alternatives considered.

## Steps
1. ...
2. ...

## Files
| File | Change |
|------|--------|

## Tests
`cargo check`, `cargo test`, manual checks.

## Risks
Risks + mitigations.
```

## Rules

- Read before edit; use unique `oldText`.
- Verify after building; don't assume success.
- Continue while steps remain; stop when verified.
- If a tool fails, read the error and adjust — don't repeat a succeeded call.
