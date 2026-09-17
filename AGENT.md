# AGENT.md — Project Context for lean

> This file is loaded on every lean session. Keep it concise and actionable. It is also mirrored to CLAUDE.md for Claude Code compatibility.

## Project Overview
- **Name:** lean
- **Purpose:** Light, fast autonomous coding assistant — single native binary, TUI-first. Streams responses over SSE, executes tools for files, shell, web search, memories, and skills. Works with any OpenAI-compatible API.
- **Status:** active (v0.6.1)
- **Repository:** https://github.com/naiih001/lean

## Tech Stack
- **Language:** Rust (edition 2021, rust-version 1.78)
- **Framework:** Ratatui (TUI), crossterm (terminal handling)
- **Package manager:** Cargo
- **Key dependencies:** tokio, reqwest, rmcp (MCP client), serde/serde_json, clap, ratatui, crossterm, anyhow

## Commands
```bash
# Build
cargo build --release

# Run
cargo run

# Test
cargo test

# Check (fast type-check)
cargo check

# Lint / Format
cargo fmt --check
cargo clippy
```

## Project Structure
```
.
├── src/
│   ├── main.rs                 # Thin CLI entry (clap Args + tui::run)
│   ├── lib.rs                  # Re-exports for compat (pub use core::agent etc)
│   ├── agent.rs                # Shim → core::agent (compat)
│   ├── llm.rs                  # Shim → integrations::llm
│   ├── mcp.rs                  # Shim → integrations::mcp
│   ├── core/                   # Agent orchestration — what it thinks
│   │   ├── mod.rs
│   │   ├── agent.rs            # run_agent* loop (928, thin)
│   │   ├── prompts.rs          # REGULAR/PLAN/ASK consts + build_system_prompt
│   │   ├── modes.rs            # Mode enum + PLAN_MODE/ASK_MODE
│   │   ├── tracker.rs          # PlanTracker + gating helpers
│   │   ├── history.rs          # prune/history_slice/build_user_content
│   │   └── context.rs          # AGENT.md/MEMORY.md loading
│   ├── guards/                 # Safety — what blocks
│   │   ├── mod.rs
│   │   ├── allowlist.rs        # Shared JSON helper
│   │   ├── bash.rs             # Severity/Risk/analyze
│   │   ├── dir.rs              # dir_guard
│   │   ├── approval.rs
│   │   └── sudo.rs
│   ├── tools/                  # Tool impls — what it does
│   │   ├── mod.rs              # execute_tool dispatcher
│   │   ├── fs.rs               # read/write/edit + truncate
│   │   ├── bash.rs             # run_bash + sudo -S
│   │   ├── search.rs           # grep/find/ls
│   │   ├── web.rs              # web_search/fetch
│   │   └── subagent.rs         # run_subagent
│   ├── integrations/           # External systems — who it talks to
│   │   ├── llm/{client,retry,schema,sse}.rs
│   │   ├── mcp/{config,registry,transport}.rs + mod.rs
│   │   ├── models/{provider,config}.rs
│   │   ├── herdr/mod.rs
│   │   ├── observer/mod.rs
│   │   └── dictate/mod.rs
│   ├── services/               # Local state — what it remembers
│   │   ├── session/mod.rs
│   │   ├── memory/{store,recall}.rs
│   │   ├── skills/{loader,catalog}.rs
│   │   ├── agents/{catalog,runtime}.rs
│   │   └── question/{wizard}.rs + mod.rs
│   ├── tui/                    # Terminal UI — what the user sees
│   │   ├── mod.rs              # RunOpts + app_loop (still thick, to thin)
│   │   ├── layout/{mod,wrap}.rs
│   │   ├── widgets/{messages,header,subagents,input}.rs + mod.rs
│   │   ├── theme/mod.rs
│   │   └── markdown/mod.rs
│   └── support/
│       └── telemetry.rs
├── skills/                     # Built-in skills (lean-config, plan)
├── docs/architecture.md        # 6-folder vocabulary (this refactor)
├── src/README.md               # (optional) per-folder one-liners
├── .lean/                      # Sessions, plans, local config
├── .github/                    # GitHub Actions workflows
└── install.sh / install.ps1
```

## Conventions
- **Style:** `cargo fmt`, `cargo clippy`
- **Commits:** Conventional commits (e.g., `chore: release v0.3.0`)
- **Branching:** main + feature branches
- **Release:** Tag `v*.*.*` triggers GitHub Actions release workflow
- **Do / Don't:** Read before edit; smallest change that solves the problem; verify once with `cargo check`

## Architecture Notes
- **Flow:** TUI → agent loop → tools/MCP/LLM
- **Streaming:** SSE for LLM responses
- **Steps:** Up to 100 steps per turn
- **MCP:** Supported via rmcp (client, child-process transport, streamable HTTP)
- **Sessions:** Per-CWD in `~/.lean/sessions`
- **Guards:** Bash allowlist with glob matching; working-directory confinement

## Gotchas
- Requires `OPENAI_API_KEY` or `OPENCODE_API_KEY` in `.env`
- `.env` is gitignored — never commit secrets
- Rust 1.78+ required
- MCP server config in `mcp.json`
- `libssl3` may be needed on Linux for reqwest
