# AGENT.md — Project Context for lean

> This file is loaded on every lean session. Keep it concise and actionable. It is also mirrored to CLAUDE.md for Claude Code compatibility.

## Project Overview
- **Name:** lean
- **Purpose:** Light, fast autonomous coding assistant — single native binary, TUI-first. Streams responses over SSE, executes tools for files, shell, web search, memories, and skills. Works with any OpenAI-compatible API.
- **Status:** active (v0.5.0)
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
│   ├── main.rs          # Entry point
│   ├── lib.rs           # Library root
│   ├── agent.rs         # Agent loop, tool-call routing
│   ├── tui/
│   │   ├── mod.rs       # TUI main
│   │   └── markdown.rs  # Markdown rendering
│   ├── mcp.rs           # MCP client
│   ├── llm.rs           # LLM client (SSE streaming)
│   ├── tools.rs         # Tool definitions
│   ├── models.rs        # Data models
│   ├── session.rs       # Session management
│   ├── context.rs       # Context management
│   ├── memory.rs        # Memory system
│   ├── skills.rs        # Skills system
│   ├── observer.rs      # Observer
│   ├── bash_guard.rs    # Bash allowlist guard
│   ├── dir_guard.rs     # Working directory confinement
│   ├── approval.rs      # Approval system
│   ├── question.rs      # User interaction
│   ├── theme.rs         # Theme
│   └── telemetry.rs     # Telemetry
├── skills/              # Built-in skills (lean-config, plan)
├── .lean/               # Sessions, plans, local config
├── .github/             # GitHub Actions workflows
└── install.sh / install.ps1  # Install scripts
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
