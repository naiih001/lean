# Architecture — lean 1:1 Foldered Layout

> **Invariant:** Every file has a single responsibility (`wc -l src/**/*.rs | awk '$1>500'` must be empty, except `tui/mod.rs` shim). 6 top-level folders map to 6 failure domains.

## Folder Vocabulary

| Folder | Purpose | Failure Domain | Owner | Key Modules |
|---|---|---|---|---|
| `core` | **What the agent thinks** — prompt assembly, mode state, planning gate, history pruning. No I/O, pure logic. | Prompt drift, mode confusion | Agent loop | `core/prompts` (REGULAR/PLAN/ASK), `core/modes` (Mode atomics), `core/tracker` (PlanTracker), `core/context` (AGENT.md), `core/history` (prune/build_user_content), `core/agent` (run_agent loop) |
| `guards` | **What blocks** — safety confinement. Leaf crate, no `tools` dep. | Allowlist persistence, false positives | Safety | `guards/allowlist` (shared JSON), `guards/bash` (Severity/Risk/analyze), `guards/dir` (analyze_path), `guards/approval` + `sudo` (OnceLock queues) |
| `tools` | **What it does** — tool implementations called by the loop. Depends on `guards` + `integrations`. | Tool dispatch, output truncation | Tools | `tools/fs` (read/write/edit, truncate_output), `tools/bash` (run_bash + sudo -S), `tools/search` (grep/find/ls), `tools/web` (web_search/fetch), `tools/subagent` (run_subagent), `tools/mod` (execute_tool + guard orchestration) |
| `integrations` | **Who it talks to** — external systems, each subfolder one integration. | API drift, transport | External | `integrations/llm/{client,retry,schema,sse}` (Client, RetryPolicy, tool schemas, SSE), `integrations/mcp/{config,registry,transport}` (load_config, LiveEntry, call_tool), `integrations/models/{provider,config}` (Provider/ApiMode, ModelsConfig), `integrations/herdr` + `dictate` (report, transcribe) |
| `services` | **Local state** — catalogs, each folder one service. | Disk corruption, cache staleness | State | `services/session` (Session), `services/skills/{loader,catalog}` (Skill), `services/agents/{catalog,runtime}` (Agent, SubagentStatus), `services/question/{wizard}` (Wizard) |
| `tui` | **What the user sees** — terminal UI, each file one layer. | Rendering, input latency | UI | `tui/layout/{wrap,mod}` (hard_wrap, context_window), `tui/widgets/{messages,header,subagents,input}` (render_lines, draw_header, etc), `tui/theme` (Ashen), `tui/markdown` (render_markdown), `tui/mod` (RunOpts, app_loop) |
| `support` | **Leaf support** — not a failure domain, just telemetry. | — | — | `support/telemetry` (hotkey_usage.json) |

## Why 6 folders

`core` = loop, `guards` = blocks, `tools` = does, `integrations` = talks to, `services` = state, `tui` = sees. Each maps to a failure domain and a review owner, so `git log -- src/core/` is prompt changes, `src/guards/` is safety, `src/tools/` is tool behavior, etc. `lib.rs` keeps `pub use` compat re-exports (`crate::agent` → `crate::core::agent`, `crate::bash_guard` → `crate::guards::bash`, etc.) so `cargo check` stays green during migration.

## 1:1 Rule

No file exceeds ~350 lines after split except `tui/mod.rs` (~3.1k, still single topic: event loop) and `core/agent.rs` (~928, loop). Linter: `wc -l src/**/*.rs src/**/**/*.rs | sort -n` — max <550, p50 ~150. New code must stay within the file's single topic; add a new file instead of growing an existing one.

## Flow

`TUI (tui::run) → core::agent::run_agent* → core::tracker (gate) → core::history (prune) → integrations::llm::Client::chat/responses_url → SSE (integrations::llm::sse) → tools::execute_tool → guards::* (approve) → integrations::mcp::call_tool / services::* → loop`
