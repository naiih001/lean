# lean

Light, fast autonomous coding assistant — single native binary, TUI-first.

`lean` runs in your terminal, streams LLM responses over SSE, and executes a small set of high-signal tools (files, bash, web search, memories, skills). No Electron, no Node — just `cargo build --release`.

> Default model: `mimo-v2.5-free` · Default provider: `OPENCODE_BASE_URL=http://127.0.0.1:8080/zen/v1` (OpenAI-compatible)

---

## Features

- **Single binary** — `7 MB` release build, no runtime deps
- **TUI** — chat history + tool trace + streaming output, multiline input, history, `/model /clear /help`
- **Agent loop** — up to 100 tool steps,SSE streaming, tool-call routing, memory + skills
- **Guards** — bash allowlist (`wildmatch`) + CWD confinement (`dir_guard`) with approval UI
- **Sessions** — per-CWD persisted sessions, resume with `--continue` / `--resume <id>`, Tab toggles `this dir` ↔ `all`
- **Memory** — `remember` / `search_memory` / `consolidate_memory` (Jaccard >0.75 dedup)
- **Skills** — `SKILL.md` discovery from `skills/`, `.lean/skills/`, `~/.agents/skills`

---

## Install & Run

### Quick start (from source)

```bash
cargo build --release
./target/release/lean --model mimo-v2.5-free
```

### Development

```bash
cargo run -- --model mimo-v2.5-free
cargo run -- --help
```

### Pre-built binary (after GitHub Release)

```bash
# from Releases page — download `lean` for your platform
chmod +x lean
./lean --help
```

---

## TUI

| Area | What it shows |
|------|---------------|
| **List** | Chat history, tool calls (`read_file` → result), streaming LLM output (2000 char cap, truncated notice) |
| **Input** | Multiline (`Enter` newline, `Ctrl+Enter` send), history `↑/↓`, `/help /model /clear /exit`, `Esc` quit |
| **Overlays** | Approval (`bash`/`dir_guard`), Sessions picker, Allowlist editor |
| **Footer** | `model | cwd | tokens / chars` (API usage + fallback) |

Single interactive mode. Sessions are persisted to `~/.lean/sessions/*.json` (pruned to 50 messages).

---

## Configuration

Precedence: **CLI flag > env var > `.env` > default**

Env vars (loaded via `dotenvy`):

| Var | Default | Purpose |
|-----|---------|---------|
| `OPENCODE_API_KEY` / `OPENAI_API_KEY` | `sk-test` | LLM API key |
| `OPENCODE_BASE_URL` | `http://127.0.0.1:8080/zen/v1` | OpenAI-compatible base URL |
| `EXA_API_KEY` | *(none)* | `web_search` via Exa; falls back to DuckDuckGo |

CLI flags:

```bash
lean --model mimo-v2.5-free     # default
lean --continue                 # resume most recent session
lean --resume <id>              # resume by prefix
lean --no-session               # disable persistence
lean --bash-guard-disabled
lean --dir-guard-disabled
```

Other persistent configs:

- `~/.lean/allowlist.json` — bash guard allowlist (globs like `git status*`)
- `~/.lean/dir_allowlist.json` — dir guard allowlist
- `~/.lean/memory.json` — persistent memories
- `./.env` — project-local env (gitignored)

---

## Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read a file (2000 lines / 50 KB cap, truncated for LLM to 2 KB) |
| `write_file` | Write file (creates parent dirs) |
| `edit_file` | Unique `oldText` → `newText` replacement |
| `bash` | `bash -c <command>` (tail 50 KB, approval-gated) |
| `web_search` | Exa → DuckDuckGo fallback |
| `read_skill` | Load a `SKILL.md` by name |
| `remember` / `search_memory` / `recall_memory` / `list_memories` / `forget_memory` / `consolidate_memory` / `memory_stats` | Persistent memory |

---

## Skills

Drop a `SKILL.md` (with frontmatter) in any of:

- `skills/` (repo)
- `.lean/skills/` (project)
- `~/.agents/skills` (global)

Scanned with 60s cache. `using-superpowers` is filtered out. Loaded via `read_skill`.

See `skills/lean-config/SKILL.md` for self-configuration (model, keys, guards, sessions).

---

## MCP (Model Context Protocol) — Client

`lean` is an MCP **client**. It discovers tools from external MCP servers and merges them into the LLM tool list as `server__tool` (namespaced). Every MCP call is gated by the same approval overlay (`HIGH`, `[a]/[A]/Esc`).

**Config files (merged, project overrides global):**

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

* `command`/`args`/`env` → stdio (spawn child via `TokioChildProcess`)
* `url`/`headers` → Streamable HTTP (SSE internally, via `rmcp` `StreamableHttpClientTransport`); legacy HTTP+SSE is not needed — Streamable HTTP covers it
* `${VAR}` / `$VAR` expanded from env (dotenvy already loaded)
* Transports: `rmcp` `transport-child-process` + `transport-streamable-http-client-reqwest`
* `Authorization: Bearer …` is split into `auth_header` automatically; other headers go to `custom_headers`

**Lifecycle:** Eager connect at startup (15s timeout). Check status via TUI `/mcp` overlay: `connected`/`connecting`/`error` + tool count + reconnect (`Enter`/`r`).

**Example:** copy `mcp.json.example`:

```bash
mkdir -p ~/.lean
cp mcp.json.example ~/.lean/mcp.json
# set token then
cargo run -- --model mimo-v2.5-free
# in TUI: /mcp  — should show github connected
```

**OAuth:** For remote HTTP servers, supply a bearer token via `headers.Authorization` (e.g. `Bearer ${MCP_TOKEN}`). Full OAuth flow is out of scope for v1 — bring your own token.

---

## Development

```bash
cargo check
cargo build --release
```

No tests/CI for v1.

Project layout:

```
src/
  main.rs       # CLI args
  tui/          # ratatui + crossterm UI
  agent.rs      # agent loop (SSE, tool routing)
  llm.rs        # OpenAI-compatible client + tool defs
  tools.rs      # tool execution
  bash_guard.rs # wildmatch allowlist
  dir_guard.rs  # CWD confinement
  session.rs    # persistence
  memory.rs     # memory store
  skills.rs     # SKILL.md discovery
```

---

## License

MIT — see [LICENSE](LICENSE).
