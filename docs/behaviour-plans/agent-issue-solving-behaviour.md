# Agent Issue-Solving Behaviour Design

Status: Proposed

Scope: agent behaviour, prompts, tracker state, tool feedback, and eval design.

Non-scope: TUI redesign, provider/model selection changes, MCP expansion, and implementation patches.

## Problem

`lean` currently presents as model-agnostically weak at simple coding work: changing models does not reliably make it inspect the right files, make a useful edit, recover from failed tool calls, or verify the result. That points away from `src/main.rs` as the primary failure domain.

`src/main.rs` is a thin entrypoint: it parses CLI flags, resolves the model alias, and starts the TUI. The issue-solving behaviour is shaped by the agent loop and its supporting contracts:

- `src/core/prompts.rs` defines the behavioural instructions.
- `src/core/tracker.rs` decides focus, plan/ask gating, and completion signals.
- `src/core/agent.rs` runs the loop, routes tools, and decides whether to continue.
- `src/core/history.rs` controls context slicing and tool-output continuity.
- `src/integrations/llm/schema.rs` defines the tool interface exposed to models.
- `src/tools/*` turns model tool calls into filesystem, shell, search, web, and subagent actions.

The current behaviour includes brittle constraints for issue work:

- It tells the model to make "one pass" and not re-read the same file.
- It tells the model not to edit the same file twice.
- It permits completion based on text such as "All done" instead of evidence.
- It does not distinguish a successful fix from a failed edit, blocked tool, or unresolved compiler/test error.
- It relies on prompt compliance where stateful loop control is needed.

The desired design is an evidence-driven issue-solving loop: the agent should keep working until it has inspected relevant context, taken the necessary action, verified the result when it mutates files, and produced a grounded final summary.

## Goals

The agent should reliably handle small and medium coding issues without needing a perfect model.

The behaviour should make these outcomes normal:

- Inspect the repo before editing.
- Diagnose from files and tool output, not guesses.
- Make the smallest useful change, while allowing multiple edits when needed.
- Recover from failed edits, failed commands, blocked tools, malformed arguments, and compiler/test errors.
- Verify after mutation with the narrowest relevant check available.
- Stop only when there is evidence that the issue is solved or when it can clearly explain why it cannot proceed.
- Produce concise final summaries that state what changed, what was verified, and any residual risk.
- Provide deterministic offline evals so behaviour regressions are caught without relying on live model calls.

## Behaviour Contract

### Request Classification

At the start of each turn, the agent loop should classify the user request into one of these behavioural modes:

| Mode | Examples | Required behaviour |
|---|---|---|
| Conversational | greetings, thanks, casual remarks | Reply briefly with no tools. |
| Read-only | explain code, inspect files, answer architecture questions | Gather evidence from relevant files/tools, then answer. No mutation. |
| Issue-solving | fix, add, update, remove, refactor, make tests pass | Inspect, diagnose, mutate if needed, verify, summarize. |
| Planning | user asks for a plan/design/spec or PLAN mode is active | Discover read-only facts, ask only necessary questions, produce plan. |

If the mode is ambiguous, the agent should first gather discoverable facts from the repo when safe. It should ask the user only when the decision is a product/scope preference that cannot be inferred from local context.

### Issue-Solving Lifecycle

For issue-solving requests, the loop should follow this lifecycle:

1. Understand
   - Identify likely files, commands, or error sources.
   - Read relevant files before editing.
   - Use search tools when the target is not obvious.

2. Diagnose
   - Form a concrete hypothesis from file contents, errors, or test output.
   - Avoid summarizing as complete before at least one meaningful diagnostic action.

3. Act
   - Edit or write only the files needed for the request.
   - Prefer precise edits for small changes.
   - Allow repeated reads or edits when the previous attempt failed or new information appears.

4. Verify
   - Run the smallest relevant check after mutation.
   - Use project conventions where discoverable, such as `cargo check`, focused tests, or existing scripts.
   - If verification cannot run, state the exact blocker and any lower-confidence evidence.

5. Summarize
   - State changed behaviour or files at a high level.
   - State verification result.
   - State unresolved risk only when present.

### Completion Rules

Completion should be state-based, not phrase-based.

For mutation tasks, the agent may complete only when one of these is true:

- It successfully mutated at least one relevant file and ran a relevant verification that passed.
- It successfully mutated at least one relevant file, attempted verification, and can explain a concrete environmental blocker.
- It inspected the request and determined no mutation was needed, with file/tool evidence.
- It is blocked by missing user input, permission, unavailable dependency, or repeated unrecoverable tool failure, and clearly states the blocker.

For read-only tasks, the agent may complete only after it has either:

- Inspected relevant local context or tool output, or
- Determined the answer is independent of the repo and can be answered directly.

The following should never be sufficient to complete issue-solving work by itself:

- "All done."
- "Here's what I did."
- A plan without edits when the user asked for implementation.
- A summary after a failed edit or failed verification.
- A response that says it will inspect or verify later.

### Failure Recovery

The loop should treat tool failures as new evidence and continue with a targeted next step.

| Failure | Required next behaviour |
|---|---|
| `edit` old text not found | Re-read or search the target, then retry with current content. |
| malformed tool arguments | Retry with valid arguments; do not summarize as complete. |
| unknown tool | Use an available equivalent tool or explain only if no equivalent exists. |
| blocked tool | Use an allowed alternative, request approval when appropriate, or explain the blocker. |
| compiler/test failure | Read the error, diagnose the likely cause, and continue editing or inspecting. |
| command unavailable | Find project-local alternative or explain missing dependency. |
| truncated output | Narrow the command/search or read the relevant file directly. |
| no tool call after task request | Focus message should force a concrete tool call unless the task is genuinely conversational. |

Repeated identical failed actions should be avoided. After a failed action, the next action should change at least one of: file target, search query, command, edit range, or hypothesis.

## Subsystem Design

### Prompt Contract

`src/core/prompts.rs` should describe the desired workflow without over-constraining useful iteration.

The regular prompt should keep these rules:

- Be concise.
- Read relevant files before editing.
- Prefer small changes.
- Verify after mutation.
- Ask only when local discovery cannot answer the ambiguity.

The regular prompt should remove or soften these rules:

- "one pass"
- "don't re-read the same file"
- "don't edit the same file twice"
- "run ONE minimal check once only"

The replacement should make the distinction explicit:

- Do not repeat successful actions without reason.
- Do repeat inspection or edits when a prior attempt failed, context changed, or verification exposed a new issue.
- Stop only after the completion rules are satisfied.

### Tracker State

`src/core/tracker.rs` should maintain enough turn state to make completion decisions evidence-driven.

Useful tracked facts:

- Request mode: conversational, read-only, issue-solving, planning.
- Whether relevant files were inspected.
- Whether any mutation was attempted.
- Whether any mutation succeeded.
- Whether any tool failed.
- Whether verification was attempted.
- Whether verification passed, failed, or was blocked.
- Recent failed action signatures to discourage identical retries.
- Whether the agent has a concrete blocker requiring user input.

Focus messages should be generated from this state:

- If no inspection happened for an issue task, require file search/read.
- If mutation succeeded but verification did not happen, require verification.
- If verification failed, require diagnosis from the error.
- If an edit failed, require re-reading/searching before retry.
- If the task is complete by state, allow final summary.

### Agent Loop

`src/core/agent.rs` should use tracker state to decide whether to continue.

The no-tool-call branch should not complete issue-solving tasks just because text includes a completion phrase. It should complete only when tracker state permits completion.

Tool results should be classified before being added back into the loop:

- success
- failure
- blocked
- verification success
- verification failure
- mutation success
- mutation failure
- read/search evidence

The agent should not require model self-reporting to know these facts. The loop can infer many of them from tool name and result text.

### Tool Interface

`src/integrations/llm/schema.rs` should present tools in a way that encourages reliable behaviour:

- `read`: read a file.
- `grep`: search file contents.
- `find`: locate files.
- `edit`: replace unique text; if it fails, inspect current content and retry.
- `bash`: run commands; use narrow verification after edits.

Tool descriptions should be short but include recovery expectations where it matters, especially for `edit` and `bash`.

### History and Context

`src/core/history.rs` should preserve enough recent tool context for recovery:

- Do not drop the most recent failed tool output if it is needed for the next step.
- Avoid slicing history in a way that leaves tool outputs orphaned.
- Keep verification output and edit failures visible long enough for the model to act on them.

The current orphan-dropping behaviour is necessary for API correctness, but issue-solving quality depends on not losing the diagnostic information too early.

## Eval Design

The project should have deterministic offline evals and optional live evals.

### Offline Evals

Offline evals should not call a real LLM. They should simulate model turns and tool results to test the agent loop's state machine, completion rules, and recovery prompts.

Each fixture should define:

- Initial user request.
- Mock file tree or scripted tool outputs.
- Scripted model outputs/tool calls.
- Expected tool sequence or state transitions.
- Pass/fail assertion.
- Human-readable failure reason.

Offline evals should be runnable in normal CI because they do not need API keys or network.

### Live Evals

Live evals should call a configured model and run against small temporary fixture repositories.

Live evals should be opt-in because they require credentials, network, and model cost. They are useful for comparing providers, but they should not be the only protection against regressions.

### Fixture Categories

The first eval suite should include at least these fixtures:

| Fixture | Purpose | Pass condition |
|---|---|---|
| simple file edit | Basic issue-solving path | Reads target, edits it, verifies or explains no check needed. |
| compile error fix | Recovery from `cargo check` failure | Reads error, edits relevant Rust file, reruns check successfully. |
| failed unique edit recovery | Handles stale `oldText` | Failed edit is followed by read/search and corrected edit. |
| multi-file diagnosis | Finds the real file from symptoms | Uses search/read before editing the correct file. |
| verification required after mutation | Prevents premature summary | Does not finish after edit until verification is attempted. |
| blocked command recovery | Handles guard/tool denial | Chooses allowed alternative or clearly reports blocker. |
| malformed tool call recovery | Handles invalid args | Retries with valid tool args instead of stopping. |

### Eval Result Shape

Each eval result should report:

- fixture name
- pass/fail
- request mode
- tool calls made
- mutations attempted and succeeded
- verification attempted and result
- final assistant text
- failure reason, when failed

The result should be easy to display in the terminal and easy to assert in tests.

## Acceptance Criteria

The behaviour redesign is successful when:

- `src/main.rs` remains a thin entrypoint; it is not treated as the behavioural root cause.
- The regular issue-solving path is stateful and evidence-driven.
- A failed edit cannot lead directly to a final success summary.
- A failed verification cannot lead directly to a final success summary.
- Mutation tasks cannot complete from text alone.
- The prompt allows useful iteration while discouraging repeated successful work.
- Offline evals cover the core failure modes without network or model dependencies.
- Live evals are optional and used for model comparison, not CI correctness.

## Implementation Order

This design should be implemented in small behaviour-first slices:

1. Define tracker state and completion rules.
2. Update regular prompt wording to match the new contract.
3. Classify tool outcomes in the agent loop.
4. Use tracker state to generate focus messages and stop decisions.
5. Add offline eval fixtures for failure recovery and completion gating.
6. Add optional live eval command only after offline evals prove the loop contract.

## Risks

Over-tight completion rules could make the agent continue too long. Mitigation: keep explicit blocked states and permit concise summaries when verification is impossible for a concrete reason.

Over-broad tool-result classification could misread ordinary command output as failure. Mitigation: start with conservative patterns and assert them through offline evals.

Prompt changes alone could appear to help but remain model-dependent. Mitigation: move the critical completion and recovery requirements into tracker state and evals.

Live evals could become flaky. Mitigation: keep them opt-in and make offline evals the primary regression suite.

## Open Decisions

The implementation phase should decide exact names for tracker fields and eval modules based on surrounding code style.

The implementation phase should decide whether live evals are exposed through the main CLI immediately or kept as an internal developer command until the offline harness is stable.
