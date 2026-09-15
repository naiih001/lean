# lean `src/` — 1:1 Foldered Layout

> Every file has a single responsibility. `cargo check` gate after each move. See `docs/architecture.md` for the 6-folder vocabulary.

```
src/
├── main.rs              # Thin CLI entry — clap Args + tui::run, uses lean:: crate
├── lib.rs               # Foldered mods + `pub use` compat shims (crate::agent → crate::core::agent, etc.)
├── core/                # What the agent thinks — pure loop, no I/O
│   ├── agent.rs         # run_agent* (thin loop, 928)
│   ├── prompts.rs       # REGULAR/PLAN/ASK + build_system_prompt
│   ├── modes.rs         # Mode enum + PLAN_MODE/ASK_MODE
│   ├── tracker.rs       # PlanTracker + gating (is_mutating, is_readonly_bash)
│   ├── history.rs       # prune/history_slice/build_user_content
│   └── context.rs       # AGENT.md/MEMORY.md loading
├── guards/              # What blocks — leaf, no tools dep
│   ├── allowlist.rs     # Shared JSON helper (load/save)
│   ├── bash.rs          # Severity/Risk/analyze
│   ├── dir.rs           # dir_guard
│   ├── approval.rs      # OnceLock queue
│   └── sudo.rs          # OnceLock queue
├── tools/               # What it does — tool impls, depends on guards+integrations
│   ├── fs.rs            # truncate_output, read/write/edit
│   ├── bash.rs          # run_bash + sudo -S
│   ├── search.rs        # grep/find/ls
│   ├── web.rs           # web_search/fetch
│   ├── subagent.rs      # run_subagent
│   └── mod.rs           # execute_tool dispatcher + guard orchestration
├── integrations/        # Who it talks to — each subfolder one external system
│   ├── llm/{client,retry,schema,sse}.rs
│   ├── mcp/{config,registry,transport}.rs
│   ├── models/{provider,config}.rs
│   ├── herdr/mod.rs
│   ├── observer/mod.rs
│   └── dictate/mod.rs   # kept single (179) unless audio expansion
├── services/            # What it remembers — local state/catalogs
│   ├── session/mod.rs
│   ├── memory/{store,recall}.rs
│   ├── skills/{loader,catalog}.rs
│   ├── agents/{catalog,runtime}.rs
│   └── question/{wizard}.rs
├── tui/                 # What the user sees — each file one layer
│   ├── mod.rs           # RunOpts + app_loop (still 6.7k, to thin to ~3k)
│   ├── layout/{mod,wrap}.rs
│   ├── widgets/{messages,header,subagents,input}.rs
│   ├── theme/mod.rs
│   └── markdown/mod.rs
└── support/telemetry.rs # Hotkey usage, not a failure domain
```

**Compat:** `src/agent.rs`, `src/llm.rs`, `src/mcp.rs` are shims (`pub use crate::core::agent::*` etc) so `crate::agent::AgentEvent` still works via `crate::core::agent`. `src/lib.rs` also has `pub use` for `crate::bash_guard` → `crate::guards::bash`, etc. New code should use `crate::core::*`, `crate::guards::*`, `crate::tools::*`, `crate::integrations::*`, `crate::services::*`, `crate::tui::*`.

**1:1 lint:** `wc -l src/**/*.rs src/**/**/*.rs | sort -n` — max <550 except `tui/mod.rs` (event loop) and `core/agent.rs` (loop, 928). See `docs/architecture.md` for folder purpose.
