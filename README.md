# lean

Light, fast autonomous coding assistant — single native binary, TUI-first.

`lean` runs in your terminal, streams responses over SSE, and executes a focused set of tools for files, shell, web search, memories, and skills. No Electron, no Node — just `cargo build --release`.

Works with any OpenAI-compatible API.

---

## Features

- **Single binary** — no runtime dependencies, fast startup
- **TUI** — chat history, tool trace, and streaming output with multiline input, history navigation, and slash commands
- **Agent loop** — SSE streaming, tool-call routing, and up to 100 steps per turn with memory and skills integration
- **Guards** — bash allowlist with glob matching and working-directory confinement, both with approval UI
- **Sessions** — per-directory persisted sessions with resume support
- **Memory** — persistent memory with search, recall, and automatic deduplication
- **Skills** — extensible `SKILL.md` system for custom capabilities
- **MCP** — Model Context Protocol client for external tool servers

---

## Installation

### Prerequisites

- Rust toolchain (stable)

#### Optional (MCP stdio only)

- Node.js 20+ and `npx` — only if you use MCP servers with `"command": "npx"` (e.g. `@modelcontextprotocol/server-github`). HTTP MCP (`"url": "https://..."`) needs no extra install.

### Quick install (recommended)

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
# pin version: LEAN_VERSION=v0.2.0 curl -fsSL https://raw.githubusercontent.com/naiih001/lean/main/install.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
```

### Prebuilt binaries

| Platform | Asset | Notes |
|---|---|---|
| Linux x86_64 | `lean-linux-x86_64.tar.gz` | Requires `libssl3` + `ca-certificates` |
| macOS x86_64 | `lean-macos-x86_64.tar.gz` | Apple Silicon via Rosetta (arm64 via build from source) |
| Windows x86_64 | `lean-windows-x86_64.zip` | No extra deps |

Download from the [Releases](https://github.com/naiih001/lean/releases) page. Each asset has a `.sha256` checksum.

### Build from source

```bash
cargo build --release
./target/release/lean --help
```

Prerequisites: Rust stable 1.78+, (Linux) `libssl-dev` / `libssl3`, (MCP stdio) Node 20+ & `npx`.

### Development

```bash
cargo run -- --help
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
| **Input** | Multiline editing (`Enter` for newline, `Ctrl+Enter` to send), history with `↑`/`↓`, slash commands, `Esc` to quit |
| **Overlays** | Approval prompts, session picker, and allowlist editor |
| **Footer** | Current model, working directory, and token usage |

**Slash commands:** `/help` `/model` `/clear` `/exit` `/mcp`

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
| `./.env` | Project-local environment variables |

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
  agent.rs       # Agent loop (SSE, tool routing)
  llm.rs         # OpenAI-compatible client and tool definitions
  tools.rs       # Tool execution
  bash_guard.rs  # Bash allowlist guard
  dir_guard.rs   # Directory confinement guard
  session.rs     # Session persistence
  memory.rs      # Memory store
  skills.rs      # Skill discovery
```

---

## License

MIT — see [LICENSE](LICENSE).
