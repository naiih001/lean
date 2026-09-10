# lean

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/naiih001/lean?label=release)](https://github.com/naiih001/lean/releases)
[![CI](https://github.com/naiih001/lean/actions/workflows/release.yml/badge.svg)](https://github.com/naiih001/lean/actions)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange.svg)](https://www.rust-lang.org)

Light, fast autonomous coding assistant — single native binary, TUI-first.

`lean` runs in your terminal, streams responses over SSE, and executes a focused set of tools for files, shell, web search, memories, and skills. No Electron, no Node — just `cargo build --release`.

Works with any OpenAI-compatible API.

<!-- Assets placeholders — uncomment and replace when ready
<p align="center">
  <img src="assets/demo.gif" alt="lean TUI demo" width="780" />
</p>
<p align="center">
  <img src="assets/arch.svg" alt="lean architecture: TUI → agent → tools/MCP/LLM" width="780" />
</p>
-->

## Contents

- [Features](#features)
- [Installation](#installation)
  - [Quick install](#quick-install-recommended)
  - [Prebuilt binaries](#prebuilt-binaries)
  - [Build from source](#build-from-source)
  - [Prerequisites](#prerequisites)
- [Usage](#usage)
- [Configuration](#configuration)
- [Tools](#tools)
- [Skills](#skills)
- [MCP (Model Context Protocol)](#mcp-model-context-protocol)
- [Development](#development)
- [FAQ](#faq)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)
- [Security](#security)
- [Changelog](#changelog)
- [License](#license)

---

## Features

- **Single binary** — no runtime dependencies, fast startup
- **TUI** — chat history, tool trace, and streaming output with multiline input, history navigation, and slash commands
- **Agent loop** — SSE streaming, tool-call routing, and up to 100 steps per turn with memory and skills integration
- **Guards** — bash allowlist with glob matching and working-directory confinement, both with approval UI — `Shift+Tab` / `/auto-accept` enables session-only `AUTO` bypass (silent, full bypass for bash/dir/MCP)
- **Sessions** — per-directory persisted sessions with resume support
- **Memory** — persistent memory with search, recall, and automatic deduplication
- **Skills** — extensible `SKILL.md` system for custom capabilities
- **MCP** — Model Context Protocol client for external tool servers

---

## Installation

### Quick install (recommended)

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
# pin to a version:
LEAN_VERSION=v0.3.0 curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
# pin to a version:
$env:LEAN_VERSION="v0.3.0"; irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
```

### Prebuilt binaries

| Platform | Asset | Notes |
|---|---|---|
| Linux x86_64 | `lean-linux-x86_64.tar.gz` | Requires `libssl3` + `ca-certificates` |
| macOS x86_64 | `lean-macos-x86_64.tar.gz` | Apple Silicon via Rosetta (arm64 via build from source) |
| Windows x86_64 | `lean-windows-x86_64.zip` | No extra deps |

Download from the [Releases](https://github.com/naiih001/lean/releases) page. Each asset ships with a `.sha256` checksum verified by the installers.

### Build from source

```bash
cargo build --release
./target/release/lean --help
```

### Prerequisites

| Requirement | Version | Notes |
|---|---|---|
| Rust toolchain | 1.78+ stable | `rustup` recommended |
| Linux system libs | `libssl-dev` / `libssl3` + `ca-certificates` | Only for building/running on Linux |
| Node.js | 20+ with `npx` | Optional — only for MCP stdio servers (`"command": "npx"`) |

> HTTP MCP (`"url": "https://..."`) needs no Node. See [MCP](#mcp-model-context-protocol) for opt-in setup.

### Development

```bash
cargo run -- --help
cargo check
```

---

## Usage

Start `lean` in your project directory:

```bash
lean
```

Resume a previous session:

```bash
lean --continue              # resume most recent session
lean --resume <id>           # resume by session ID prefix
lean --no-session            # run without persistence
```

#### TUI Overview

| Area | Description |
|------|-------------|
| **Chat** | Conversation history, tool calls with results, and streaming output |
| **Input** | Auto-wrapping multiline editing (`Enter` send, `Shift+Enter` newline, `Shift+Tab` auto-accept, `Ctrl+C` clear), history with `↑`/`↓`, slash commands, `Esc` to quit |
| **Overlays** | Approval prompts, session picker, and allowlist editor |
| **Footer** | Current model, working directory, and token usage |

**Slash commands:** `/help` `/new` `/clear` `/exit` `/model` `/sessions` `/resume` `/allowlist` `/mcp` `/memory` `/auto-accept` — `Shift+Tab` toggles session-only auto-accept (`AUTO` badge)

Sessions are persisted to `~/.lean/sessions/*.json`.

---

## Configuration

Configuration precedence: **CLI flag > environment variable > `.env` > default**

### Environment Variables

Environment variables are loaded from `.env` via `dotenvy`.

| Variable | Default | Purpose |
|----------|---------|---------|
| `OPENAI_API_KEY` / `OPENCODE_API_KEY` | — | API key for the LLM provider |
| `OPENAI_BASE_URL` / `OPENCODE_BASE_URL` | `https://api.openai.com/v1` | OpenAI-compatible API base URL |
| `EXA_API_KEY` | *(none)* | API key for web search via Exa; falls back to DuckDuckGo when unset |

### CLI Flags

```bash
lean --model <model-id>        # select model
lean --continue                # resume most recent session
lean --resume <id>             # resume by session ID prefix
lean --no-session              # disable session persistence
lean --bash-guard-disabled     # disable bash allowlist guard
lean --dir-guard-disabled      # disable directory confinement guard
```

### Persistent Files

| Path | Purpose |
|------|---------|
| `~/.lean/allowlist.json` | Bash guard allowlist (glob patterns) |
| `~/.lean/dir_allowlist.json` | Directory guard allowlist |
| `~/.lean/memory.json` | Persistent memories |
| `~/.lean/sessions/` | Persisted session history (pruned to 50 messages) |
| `~/.lean/models.json` | Model aliases + provider config (see `api` field below) |
| `./.env` | Project-local environment variables |

### Model Config (`~/.lean/models.json`)

```json
{
  "default": "gpt-4o",
  "models": {
    "gpt-4o": {
      "model": "gpt-4o",
      "base_url": "https://api.openai.com/v1",
      "api_key_env": "OPENAI_API_KEY"
    },
    "gpt-5-responses": {
      "model": "gpt-5",
      "base_url": "https://api.openai.com/v1",
      "api_key_env": "OPENAI_API_KEY",
      "api": "responses"
    }
  }
}
```
- `model` — real model id sent to the API
- `base_url` — optional override (defaults to `OPENAI_BASE_URL` / `OPENCODE_BASE_URL`)
- `api_key_env` / `api_key` — env var name or inline key
- `api` — `"chat_completions"` (default, `POST /v1/chat/completions`) or `"responses"` (`POST /v1/responses`). Aliases: `chat`/`completions` → chat, `responses` → responses. Existing configs without `api` keep working as chat completions.

---

## Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents (with size limits and truncation) |
| `write_file` | Write or create files (auto-creates parent directories) |
| `edit_file` | Targeted text replacement via unique `oldText` matching |
| `bash` | Execute shell commands (approval-gated) |
| `web_search` | Web search via Exa with DuckDuckGo fallback |
| `read_skill` | Load a `SKILL.md` by name |
| `remember` / `search_memory` / `recall_memory` / `list_memories` / `forget_memory` / `consolidate_memory` / `memory_stats` | Persistent memory management |

---

## Skills

Skills extend `lean` with domain-specific instructions. Drop a `SKILL.md` file (with frontmatter) in any of:

- `skills/` — repository-level
- `.lean/skills/` — project-level
- `~/.agents/skills` — global

Skills are discovered with a 60-second cache and loaded via `read_skill`.

---

## MCP (Model Context Protocol)

> **MCP is opt-in.** No servers run by default. Create `~/.lean/mcp.json` to enable; stdio servers require Node 20+ and `npx`, HTTP servers need no extra deps.

`lean` acts as an MCP client, discovering tools from external MCP servers and exposing them as namespaced tools (`server__tool`). All MCP tool calls are gated by the approval overlay.

**Configuration files** (merged, project overrides global):

- `~/.lean/mcp.json` (global)
- `.lean/mcp.json` (project)

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_PERSONAL_ACCESS_TOKEN}" }
    },
    "remote-example": {
      "url": "https://mcp.example.com/mcp",
      "headers": { "Authorization": "Bearer ${MCP_TOKEN}" }
    }
  }
}
```

- `command` / `args` / `env` — stdio transport (spawns child process)
- `url` / `headers` — Streamable HTTP transport
- `${VAR}` / `$VAR` are expanded from the environment

**Lifecycle:** Servers connect eagerly at startup (15s timeout). Check status with `/mcp` in the TUI — shows `connected` / `connecting` / `error` with tool counts and a reconnect action.

**Setup example:**

```bash
mkdir -p ~/.lean
cp mcp.json.example ~/.lean/mcp.json
# set required tokens, then start lean
```

For remote HTTP servers, supply a bearer token via `headers.Authorization`. Full OAuth flows are out of scope for v1 — bring your own token.

---

## Development

```bash
cargo check
cargo build --release
```

**Project layout:**

```
src/
  main.rs        # CLI argument parsing
  tui/           # Terminal UI (ratatui + crossterm)
  agent.rs       # Agent loop (dual-stack: /chat/completions + /responses, SSE, tool routing)
  llm.rs         # OpenAI-compatible client, tool defs, and Responses translation
  tools.rs       # Tool execution
  bash_guard.rs  # Bash allowlist guard
  dir_guard.rs   # Directory confinement guard
  session.rs     # Session persistence
  memory.rs      # Memory store
  skills.rs      # Skill discovery
  models.rs      # Model config + ApiMode (chat vs responses)
```

---

## FAQ

**Which API does `lean` use?**
Any OpenAI-compatible endpoint. Configure via `OPENAI_API_KEY` / `OPENAI_BASE_URL` (or `OPENCODE_*` alias) and model aliases in `~/.lean/models.json`. Fresh installs default to `gpt-4o` at `https://api.openai.com/v1`. Supports both `POST /v1/chat/completions` and `POST /v1/responses` — set per-model `"api": "responses"` to use Responses API (see Model Config above).

**Does `lean` support `/v1/responses`?**
Yes. Add `"api": "responses"` to a model entry in `~/.lean/models.json` (or use alias `"api": "chat"` for chat completions). Verify with `cat ~/.lean/models.json | jq .`; switch with `lean --model <alias>` or `/model <alias>` in the TUI — footer shows `(chat)` vs `(responses)`. Both modes share the same tools and streaming UX.

**How do I switch models?**
`lean --model <alias>` for one run, or `/model <alias>` inside the TUI (Tab completes aliases). Edit `~/.lean/models.json` to add providers — `alias → { model, base_url, api_key_env }`.

**Why is `Shift+Enter` not inserting a newline?**
`Shift+Enter` uses the kitty keyboard protocol (`DISAMBIGUATE_ESCAPE_CODES`). Unsupported terminals ignore the sequence and send plain `Enter`. Try a modern terminal (Ghostty, Kitty, WezTerm, recent Alacritty) or paste multiline text — input auto-wraps pasted content the same way.

**Does `lean` send my code to the LLM?**
Only the tool outputs `lean` generates in-session (file snippets via `read_file`, bash output, etc.) plus your prompts. There is no background telemetry. Sessions stay local in `~/.lean/sessions/`.

**How are sessions scoped?**
Per working directory, persisted as `~/.lean/sessions/*.json` (pruned to 50 messages). Use `lean --continue` or `lean --resume <id>` to restore; `lean --no-session` for ephemeral runs.

**What are `@file` and `$skill` mentions?**
`@` autocompletes project files and expands contents inline on submit. `$` forces a skill (`SKILL.md`) into context. Both complete with `Tab`/`Enter`.

**How do I disable approval prompts?**
Press `Shift+Tab` (or `/auto-accept on`) to enable session-only `AUTO` (footer badge) — all guards bypassed silently until `Shift+Tab` again or session ends. Use `/auto-accept off` to re-enable.

## Troubleshooting

| Symptom | Cause / Fix |
|---------|-------------|
| `No API key found for 'OPENAI_API_KEY'` on startup | Set `OPENAI_API_KEY` (or `OPENCODE_API_KEY`) in env or `.env`. For custom providers, ensure `~/.lean/models.json` `api_key_env` points to the right var. |
| `unknown model alias '…'` | Check `~/.lean/models.json` — `default` must exist in `models`. List aliases with `/model` + Space. Validate JSON with `jq empty ~/.lean/models.json`. |
| `401 / 403` from API | Key is invalid or base URL mismatched. Confirm `OPENAI_BASE_URL` has `/v1` suffix and matches provider. |
| Linux build/run: `libssl` / `ca-certificates` errors | `sudo apt-get install libssl-dev pkg-config` (build) and `libssl3 ca-certificates` (run). Release binaries are linked against `libssl3`. |
| MCP stdio server fails to start | Requires Node 20+ and `npx` on `PATH`. HTTP MCP (`url`) needs no Node. Check `/mcp` overlay for `error` state and tool counts; verify `${VAR}` env expansion. |
| `bash` always asks for approval | Expected — `bash` and MCP tools are approval-gated (`[a]`/`[A]`/`Esc`). Add glob patterns to `~/.lean/allowlist.json` to allowlist safe commands, or press `Shift+Tab` / `/auto-accept` for session-only bypass (`AUTO` badge). |
| File edits outside project blocked | `dir_guard` confines to CWD. Use `[a]` to approve once or `[A]` to allowlist, or run with `lean --dir-guard-disabled`. |
| Empty or missing sessions | Sessions are per-CWD and pruned to 50 messages. Check `~/.lean/sessions/` and current directory. |

Still stuck? Open an issue with `lean --help` output, OS, terminal, and the error message.

---

## Contributing

Contributions welcome. For small fixes, open a PR directly. For larger changes, please open an issue first to discuss.

```bash
cargo check
cargo build --release
cargo test
bash -n install.sh   # syntax check
```

- Follow existing Rust style (`cargo fmt` where applicable); keep `lean` lean — single binary, no extra runtime deps.
- Update `CHANGELOG.md` under `## [Unreleased]` for user-facing changes.
- See [RELEASING.md](RELEASING.md) for the release ritual and cross-platform artifact matrix.

## Security

`lean` is MIT-licensed and provided as-is. If you find a security-relevant bug (e.g., guard bypass, path traversal, credential leak):

- Do not open a public issue with exploit details.
- Open a private security advisory on GitHub or email the maintainers via the repository profile.
- Session files, allowlists, and `~/.lean/` data stay local — do not paste secrets into issues. Redact API keys and tokens.

For general bugs, use the issue tracker.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes. `v0.3.0` adds input auto-wrap, `Shift+Enter` newline, `Shift+Tab` auto-accept (`/auto-accept` + `AUTO` badge), and a markdown parser fix.

---

## License

MIT — see [LICENSE](LICENSE).
