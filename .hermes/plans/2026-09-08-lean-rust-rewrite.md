# lean → Rust Rewrite — Plan (Reimagining)

**Date:** 2026-09-08  
**Current codebase:** ~1,500 LOC, 7 files in `src/` (tui 575, tools 225, agent 190, index 140, skills 130, llm 106, theme 50), Bun + TypeScript + Ink 7 + React 19 + OpenAI SDK 7.10.0, 12 commits on `main`, no remote, no tests/CI/Docker, clean tree  
**Goal:** Reimagining rewrite to Rust — single native binary replacing TS in-place, new TUI layout (list + input + footer), keep core semantics

---

## 1. Shared Understanding (Verified)

### What lean does today (from scouts)
- Minimal autonomous coding assistant CLI. Entry `src/index.ts` → `runAgent()` (`src/agent.ts` AsyncGenerator<AgentEvent>) streams `client.chat.completions.create(stream:true)` via `127.0.0.1:8080/zen/v1`, accumulates `toolCalls` by index, parallel `Promise.all(executeTool)`, max 100 steps, no human approval.
- 6 tools in `src/llm.ts`/`src/tools.ts`: `read_file` / `write_file` / `edit_file` (unique-match) / `bash` (unrestricted `bash -c`) / `web_search` (Exa→DuckDuckGo) / `read_skill`. Truncation: `truncateOutput` dual limit 2000 lines / 50KB (head for files, tail for bash), `truncateForLLM` 2KB chars for LLM, full shown in TUI (but TODO `tools.ts:69` note never surfaced).
- Skills: `src/skills.ts` scans `./skills`, `./.lean/skills`, `~/.agents/skills` for `SKILL.md` frontmatter, 60s TTL cache, local wins, catalog injected into system prompt, `using-superpowers` filtered.
- Two UIs sharing loop: Ink TUI (`src/tui.tsx` — history, streaming 40ms throttle, viewport windowing, ToolPanel memo, scroll PageUp/Down 10/Home/End, spinner) + `readline` fallback. Theme `src/theme.ts` ashen dark `#a0a0a6/#b07156/#1a1a1e`.

### User-confirmed Rust decisions
| Dimension | Decision |
|---|---|
| **Scope** | Reimagining (not 1:1), but keep core loop/tools/skills semantics |
| **TUI** | New layout: **list + input + footer**. List = chat history + tool calls + output (2000 char max retained). Input = prompt box, **keep multiline + history + slash commands** (`/help /model /clear /exit`). Footer = **model, directory (CWD), context window stats**. List **simplified**: basic scroll, no 40ms throttling/virtualization, minimal tool rendering (no collapsible/memo). |
| **Theme** | Keep ashen dark palette |
| **Context stats** | API `usage` field when available, fallback char estimate (no tiktoken v1) |
| **Tools** | Keep same 6 exactly, same head/tail & 2KB semantics, parallel |
| **LLM** | Keep identical env vars/defaults (`OPENCODE_API_KEY`/`OPENAI_API_KEY`, `OPENCODE_BASE_URL=http://127.0.0.1:8080/zen/v1`, `DEFAULT_MODEL=mimo-v2.5-free`, `EXA_API_KEY`→DuckDuckGo) |
| **Skills** | Keep identical 3 paths, 60s cache, SKILL.md frontmatter, filter |
| **Modes** | **Only interactive TUI** — remove one-shot `lean "prompt"` and `readline --simple` fallback. Keep `--model` flag. Use `dotenvy` for `.env`. |
| **Distribution** | Single binary, **replace TS in-place** in same repo (remove TS after) |
| **Stack** | No hard constraint — propose idiomatic Rust |
| **Tests/CI** | Keep minimal — no tests/CI (like now), `cargo check` only |
| **Loop** | Keep max 100 steps, parallel tool execution |

**Open minor:** Fix truncation TODO (show `… [truncated]` note in TUI) — assumed yes for Rust.

---

## 2. Goals / Non-Goals

**Goals**
- Single `lean` binary (`cargo build --release`) replacing `bun src/index.ts` with same agent capability
- New list+input+footer TUI in Rust, simpler than Ink but preserving streaming + tool visibility
- Drop Bun/Node/React dependency, keep env/skill compatibility for drop-in use
- Clean module mapping `src/*.ts` → `src/*.rs`

**Non-Goals**
- Add new tools, providers, or skill formats (deferred)
- Real token counting via tokenizers (deferred to char+API usage)
- Tests, CI, cross-platform release, Windows support (minimal v1)
- One-shot or headless mode (removed per decision)

---

## 3. Proposed Architecture

```
                ┌─────────────────────────────────┐
                │  CLI (clap) --model, --help     │
                │  dotenvy + env var resolution   │
                └──────────────┬──────────────────┘
                               ▼
                ┌─────────────────────────────────┐
                │  App (tokio)                    │
                │  agent::run_agent()             │
                │  Stream → AgentEvent            │
                │  ├─ llm::Client (reqwest SSE)   │
                │  ├─ tools::execute (parallel)   │
                │  └─ skills::catalog             │
                └──────────────┬──────────────────┘
                               ▼
                ┌─────────────────────────────────┐
                │  TUI (ratatui + crossterm)      │
                │  ┌───────────────────────────┐  │
                │  │  List (messages+tools)    │  │
                │  │  scrollable, 2000char cap │  │
                │  ├───────────────────────────┤  │
                │  │  Input (multiline+history)│  │
                │  └───────────────────────────┤  │
                │  │ Footer: model | CWD | ctx │  │
                │  └───────────────────────────┘  │
                └─────────────────────────────────┘

Data flow: Input → agent loop (buildSystemPrompt + catalog) → LLM SSE stream → yield text/reasoning → if tool_calls → tokio::join_all → truncate head/tail → tool messages → next step (100 max) → done. Events drive TUI list; footer polls model/CWD/usage.
```

---

## 4. Proposed File Layout

Replace current `src/` (keep `src/` dir, change language):

```
lean/
├── Cargo.toml
├── Cargo.lock
├── src/
│   ├── main.rs      # CLI (clap), dotenvy, tokio::main, launch TUI
│   ├── agent.rs     # SYSTEM_PROMPT, build_system_prompt, AgentEvent, run_agent (stream loop, join_all, truncate_for_llm)
│   ├── llm.rs       # Client (reqwest SSE), DEFAULT_MODEL, TOOLS schema (serde_json), OpenAI-compatible types
│   ├── tools.rs     # truncate_output(head/tail 2000/50KB), read_file/write_file/edit_file (tokio::fs), bash (tokio::process), web_search (reqwest Exa→DuckDuckGo), read_skill dispatcher
│   ├── skills.rs    # discover_skills (3 bases, 60s cache, frontmatter), get_skill_catalog, load_skill
│   ├── tui/
│   │   ├── mod.rs   # App state, event loop, key handling
│   │   ├── list.rs  # List widget (messages, tool calls+output, 2000char, basic scroll)
│   │   ├── input.rs # Multiline input, history, slash commands
│   │   └── footer.rs# Footer widget (model, cwd, ctx stats from usage+char fallback)
│   └── theme.rs     # ashen palette → ratatui Style mapping
├── .env.example     # OPENCODE_API_KEY, OPENCODE_BASE_URL, EXA_API_KEY
├── .gitignore       # add /target, keep existing
└── README.md        # updated cargo instructions
```

Remove after port: `package.json`, `bun.lock`, `tsconfig.json`, `node_modules`, old `src/*.ts`/`*.tsx`.

---

## 5. Crate Choices (Proposed Idiomatic)

| Concern | Crate | Why |
|---|---|---|
| **Async runtime** | `tokio` (full) | Agent loop streaming + parallel `join_all`, `tokio::fs/process` |
| **HTTP + SSE** | `reqwest` (json, stream) + `eventsource-stream` or `futures` | OpenAI SSE `chat.completions.create(stream:true)`, Exa/DuckDuckGo search |
| **TUI** | `ratatui` + `crossterm` | List+input+footer, replaces Ink/React, supports multiline + scroll |
| **CLI** | `clap` (derive) | `--model`, `--help`, keep parity |
| **Serialization** | `serde` + `serde_json` | Tool schemas, LLM messages |
| **Env** | `dotenvy` | Replace Bun auto `.env` |
| **Frontmatter** | `gray_matter` or manual `serde_yaml` | `SKILL.md` parsing |
| ** FS/glob** | `tokio::fs`, `walkdir`, `dirs` (homedir) | Skills discovery, file tools |

No `async-openai` unless it supports custom baseURL `127.0.0.1:8080/zen/v1` cleanly — `reqwest` gives full control for SSE + `usage` field.

---

## 6. TUI Layout (List + Input + Footer)

**Wireframe (ratatui Layout vertical):**
```
┌─ lean ─ light coding assistant ──────────────────────────┐
│ List (flex 1, scrollable)                                │
│  [user] hello                                            │
│  [assistant] streaming text …                             │
│  [tool] read_file {path:"src/main.rs"}                  │
│  └─ <output 2000char, truncated note if needed>          │
│  [tool] bash {command:"cargo check"}                     │
│  └─ <tail output>                                        │
│  … PageUp/PageDown 10 lines, Home/End, ↑/↓ scroll        │
├───────────────────────────────────────────────────────────┤
│ Input (3-5 lines, multiline)                             │
│ > █  (Enter send, Shift+Enter newline, Up/Down history, │
│   /help /model /clear /exit)                             │
├───────────────────────────────────────────────────────────┤
│ Footer: model: mimo-v2.5-free | dir: /home/naet/.../lean │
│         ctx: 1.2k/128k tokens (or 4.1k chars fallback)    │
└───────────────────────────────────────────────────────────┘
```

**Behaviors retained:** streaming text deltas appended to list, tool_start/tool_result interleaving, spinner while waiting, ashen colors via `theme.rs`.  
**Simplified away:** 40ms `scheduleFlush` throttling, viewport `termRows-6` windowing memo, `ToolPanel` collapse/memoization, `StreamingTail` virtualization — basic `ListState` + `ScrollbarState` instead.

**Input:** `tui/input.rs` handles `crossterm` key events: Enter submits, Shift+Enter inserts newline, Up/Down navigates history Vec, `/` prefix dispatches commands (help→toast, model→switch footer, clear→reset list, exit→quit).

**Footer:** `footer.rs` renders `model` (from `--model` or env default), `cwd` (`std::env::current_dir`, refreshed after each `bash` tool that may `cd`), `ctx` (`usage.prompt_tokens + usage.completion_tokens` if `stream` final chunk has `usage`, else `total_chars / 4` estimate + message count).

---

## 7. Module Ports (Key Decisions)

**agent.rs**
- `SYSTEM_PROMPT` constant + `build_system_prompt() -> String` (calls `skills::get_catalog()`).
- `enum AgentEvent { Text { delta }, Reasoning { delta }, ToolStart { name, args, id }, ToolResult { name, result, id }, Done { text } }`
- `async fn run_agent(prompt: String, model: String, max_steps: usize) -> impl Stream<Item=AgentEvent>` — loop SSE, accumulate `tool_calls` by index (serde_json Value), if none → Done, else `futures::future::join_all(execute_tool)` (parallel), `truncate_for_llm(2000)` for LLM, full 50KB/2000line for TUI (with note). Keep `truncate_for_llm` `… [truncated N chars for LLM, full shown in TUI]`.

**llm.rs**
- `const DEFAULT_MODEL: &str = "mimo-v2.5-free"`
- `struct Client { api_key, base_url, client: reqwest::Client }` with `fn new() -> Self` reading `OPENCODE_API_KEY` → `OPENAI_API_KEY` → `sk-test`, `OPENCODE_BASE_URL`.
- `TOOLS: &[Tool]` 6 definitions mirroring `src/llm.ts` JSON schemas (serde_json).
- SSE parsing: `POST /chat/completions` with `stream:true`, `eventsource-stream`, accumulating `content`, `reasoning_content`, `tool_calls`.

**tools.rs**
- `const MAX_LINES: 2000, MAX_BYTES: 50*1024` — `fn truncate_output(content: &str, strategy: Head|Tail) -> String` (line-preserving, never split mid-line, append `… [truncated: kept …]`).
- `async fn read_file(path: &str) -> Result<String>` via `tokio::fs::try_exists` + `read_to_string`, `write_file` via `create_dir_all` + `write`, `edit_file` unique-match replace (error if 0 or >1 matches), `bash` via `tokio::process::Command::new("bash").arg("-c")`, `web_search` via `reqwest` Exa→DuckDuckGo fallback, `execute_tool(name, args)` dispatcher. Ensure parent `mkdir -p` via `create_dir_all` not `$`.

**skills.rs**
- `fn discover_skills() -> Vec<Skill>` scanning `skills/`, `.lean/skills/`, `~/.agents/skills` (via `dirs::home_dir`), `SKILL.md` frontmatter (simple `---` split + name/description), 60s cache (`OnceLock` + `Instant`), local wins over global.
- `fn get_skill_catalog() -> String` filtering `using-superpowers`, `fn load_skill(name) -> String`.

**theme.rs**
- Port `ashen` constants → `ratatui::style::Color::Rgb` and semantic `Theme { accent, tool_bg, page_bg, ... }`.

---

## 8. Build / Run / Distribution

- `cargo run -- --model mimo-v2.5-free` (interactive TUI only), `cargo run -- --help`
- `cargo check` replaces `bun --check`
- `cargo build --release` → `target/release/lean` single binary (strip, ~5-10MB). Replace `package.json` scripts with `cargo` equivalents.
- `.gitignore` add `/target`, `Cargo.lock` commit (binary).
- No Docker/CI for v1.

---

## 9. Migration Steps (In-Place Replacement)

1. **Scaffold** `Cargo.toml` + `src/main.rs` stub that prints placeholder TUI (no LLM yet), verify `cargo run` in repo.
2. **Port skills + theme + tools (pure)** — no LLM needed, unit-test via `cargo check` only per minimal testing decision (manual smoke with `Bun.file` parity checks).
3. **Port llm + agent loop** with SSE, test against `127.0.0.1:8080/zen/v1` with `sk-test`, verify streaming events.
4. **Build TUI** (ratatui list+input+footer), wire `run_agent` stream into list, footer stats, input history/commands.
5. **Remove TS** — delete `src/*.ts*`, `package.json`, `bun.lock`, `tsconfig.json`, `node_modules`, update `README.md` + `.gitignore`.
6. **Smoke** — `cargo build --release` and run interactive session covering read/write/edit/bash/search/skill.

Rollback: git tag `pre-rust` before deletion, keep branch if needed.

---

## 10. Risks & Edge Cases

- **SSE compat:** custom `zen` proxy may differ from OpenAI SSE (field names `reasoning_content` vs `reasoning`). Mitigate: tolerate both, log raw chunk on parse failure.
- **Bash unrestricted:** same risk as TS — no sandbox. Document, no change per keep-6-tools decision.
- **CWD in footer:** `bash -c "cd subdir"` in child process doesn't affect parent `current_dir`. Mitigate: detect `cd` in bash args and `std::env::set_current_dir` after success, or document footer reflects launch dir only.
- **Ratatui multiline input:** harder than Ink — need custom `TextArea` handling for Shift+Enter. Use `tui-textarea` crate if needed.
- **Skill frontmatter:** naive split may break if `---` in body. Keep same simple logic as TS for parity.
- **Truncation display:** ensure TUI shows `… [truncated]` note (fixing TODO) — both for LLM and TUI views.

---

## 11. Verification Checklist (Before Merging)

- [ ] `cargo run` launches list+input+footer TUI with ashen theme, `--model` flag works, `.env` loaded via dotenvy
- [ ] Typing multiline + history + `/help /model /clear /exit` behaves like Ink version
- [ ] `read_file`/`write_file`/`edit_file`/`bash`/`web_search`/`read_skill` all work, parallel, 2000/50KB head/tail correct, note shown
- [ ] Agent streams text/reasoning, tool calls interleave, footer CWD/model/ctx update, 100 steps max
- [ ] Skills discovered from 3 paths, `using-superpowers` filtered, `read_skill` loads
- [ ] `cargo build --release` produces single binary, TS files removed, `README` updated

---

*Plan ready for review. Upon approval, implementation starts at step 1 (scaffold Cargo project).*
