# Changelog

All notable changes to `lean` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-09-10

Release-engineering lifecycle — cross-platform public GitHub Releases.

### Added

- **Cross-platform GitHub Releases** — `.github/workflows/release.yml` (tag `v*.*.*` trigger) matrix builds `x86_64-unknown-linux-gnu` (ubuntu), `x86_64-apple-darwin` (macos), `x86_64-pc-windows-msvc` (windows) via `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache`; packages `lean-linux-x86_64.tar.gz` / `lean-macos-x86_64.tar.gz` / `lean-windows-x86_64.zip` + `.sha256` and publishes via `softprops/action-gh-release@v2` with install snippet. Linux job installs `libssl-dev`/`pkg-config`; smoke `--help` on all three.
- **One-liner installers** — `install.sh` (bash, linux/macos, detects `uname -s`/`uname -m`, resolves `LEAN_VERSION` via GitHub API + redirect fallback, verifies `.sha256`, extracts to `~/.local/bin` / `$LEAN_INSTALL_DIR`) and `install.ps1` (PowerShell, Windows, `Expand-Archive` + `Get-FileHash` verify, installs to `$USERPROFILE\.lean\bin`). Both support `LEAN_VERSION=vX.Y.Z` pin.
- **RELEASING.md** — release ritual (`Cargo.toml` bump → `CHANGELOG.md` → `git tag v* && git push --tags` → watch Actions → verify installers), workflow details, preflight checklist, troubleshooting.
- **.dockerignore** — excludes `.env`/`.lean`/`.hermes`/`target`/`dist` from artifacts.
- **.github/README.md** — workflow scaffold.

### Changed

- **Cargo.toml metadata** — adds `description`, `repository`, `homepage`, `readme`, `license`, `keywords`, `categories`, `rust-version = "1.78"`, `exclude` for release hygiene.
- **README install** — replaces minimal `cargo build` block with quick-install (`curl | bash` / `irm | iex`), prebuilt binaries table (linux/macos/windows + sha256), and prerequisites (`libssl3`/`ca-certificates` on Linux, Node 20+ only for MCP stdio).
- **README MCP** — clarifies MCP is opt-in: no servers by default, `~/.lean/mcp.json` to enable, stdio requires Node 20+ & `npx`, HTTP needs no extra deps (also in Prerequisites section).

### Fixed

- **Fresh-install UX** — `src/models.rs` `template_config()` locked to OpenAI (`gpt-4o → https://api.openai.com/v1`, `api_key_env = OPENAI_API_KEY`) with `template_is_openai_default` test guard; `resolve_api_key_env` now fails fast with `No API key found for 'OPENAI_API_KEY' — set OPENAI_API_KEY …` instead of silent `sk-test` → opaque 401. Existing `~/.lean/models.json` owners with `OPENCODE_API_KEY` unaffected (`dotenvy` loads `.env` before resolve). Adds `resolve_fails_without_key` test.

## [0.2.0] - 2026-09-09

Aggregates all changes since `0.1.0` (origin/main) — 8 commits + 1 uncommitted patch. This is a minor release: new features, no breaking changes.

### Added

- **@ file references in chat** — `19a215f` (2026-09-09) — `feat: add @ file reference in chat`. Type `@` to autocomplete project files (walkdir, filtered), highlight mentions in user messages (moss/bone styling), and expand file contents inline on submit/queue. Affects `src/tui/mod.rs`.

- **Model registry via `~/.lean/models.json`** — `bf4aa6e` (2026-09-09) — `feat: model registry via ~/.lean/models.json`. New `src/models.rs` (`alias → { model, base_url, api_key, api_key_env }`) with auto-created template (`gpt-4o`) if missing, env fallback for `base_url`/`api_key`. `--model` is now an alias resolved via `models::resolve()`; `llm::Client::from_resolved()` uses the resolved provider. Sessions store the alias and warn on legacy aliases. Default alias `mimo-v2.5-free` at `http://127.0.0.1:8080/zen/v1`. Affects `src/models.rs`, `src/main.rs`, `src/llm.rs`, `src/agent.rs`, `src/tui/mod.rs`.

- **Autocomplete for model aliases on `/model`** — `360494d` (2026-09-09) — `feat: autocomplete for model aliases on /model`. `autocomplete_matches` now returns `Vec<String>` and completes `/model <prefix>` from `models.json` aliases. `/model` + space lists all aliases; Tab/Enter inserts ` /model <alias>`. Affects `src/tui/mod.rs`.

- **Force skills with `$`** — `813cb75` (2026-09-09) — `feat: add $ to force skills`. `$skill` autocomplete (`detect_skill_mention`, `list_skill_names_sync`, `skill_autocomplete_matches`) with priority over `@`-files. `expand_at_mentions` now injects skill `SKILL.md` content as forced skill context on submit/queue. `$` mentions rendered in moss, Enter/Tab handling for both `@` and `$`, placeholder updated to `@file $skill`. Affects `src/tui/mod.rs`.

- **MCP client — stdio + Streamable HTTP via `rmcp 3.2`** — `8031289` (2026-09-09) — `feat: MCP client — stdio + Streamable HTTP via rmcp, /mcp overlay and approval gate`. Lean is now an MCP **client**: discovers tools from external MCP servers and merges them as `server__tool` into the LLM tool list. Every MCP call is gated by the `HIGH` approval overlay (`[a]/[A]/Esc`). Config merged from `~/.lean/mcp.json` (global) + `.lean/mcp.json` (project) with `${VAR}`/`$VAR` expansion. Transports: stdio (`command`/`args`/`env` via `TokioChildProcess`) and Streamable HTTP (`url`/`headers`, `Authorization` split to `auth_header`). Eager connect at startup with 15s timeout; TUI `/mcp` overlay shows `connected`/`connecting`/`error`/`disabled` + tool count with `d`/`g`/`r` controls. System prompt lists MCP catalog; chat filters server banner noise. Adds `mcp.json.example`, ignores `.lean/` locally, adds `rmcp` + `http` deps. Affects `.gitignore`, `Cargo.toml`, `Cargo.lock`, `README.md`, `mcp.json.example`, `src/mcp.rs`, `src/llm.rs`, `src/tools.rs`, `src/main.rs`, `src/lib.rs`, `src/tui/mod.rs`, `src/agent.rs`.

- **Conversational vs Task mode (greeting/smalltalk short-circuit)** — *uncommitted, included in 0.2.0* (2026-09-09) — `src/agent.rs` patch (60 lines). `SYSTEM_PROMPT` split into conversational vs `TASK MODE` with explicit rules: greetings/thanks/`hi` with no task verb → 1–2 sentence warm reply and STOP (no tools, no memory search, no step listing). Adds `PlanTracker::is_conversational_goal()` heuristic (exact matches, greeting prefixes, task-verb guard), `focus_context()` conversational branch (`[FOCUS CONTEXT — conversational turn]`), and early `Done` break in `run_agent_with_history` when `tool_acc.is_empty()` and conversational. Fixes bug where `Hi` triggered 3+ tool-calling loops. Affects `src/agent.rs`.

### Fixed

- **Boxed per-tool UI with timer and queued parallel approvals** — `c0c1389` (2026-09-09) — `fix: boxed per-tool UI with timer and queued parallel approvals`. `src/approval.rs`: `Option` → `VecDeque` queue to fix race where parallel guarded tools overwrote approvals; adds `queue_len`/`pending_count` FIFO. `src/agent.rs`: per-tool wall-clock `Instant` → `elapsed_ms` on `ToolResult` (preserves `join_all` parallelism). `src/tui/mod.rs`: one live box per tool (`┌─ name · timer ─┐` / `│` / `└─`) with border color by state (charcoal running, moss success, ember error), timer in header (`ms`/`s`), truncated diff-aware body, in-place update via `tool_id`, approval overlay `[1/N]` queued count.

### Changed

- **Model-registry integration merge** — `96c3d30` (2026-09-09) — `Merge branch 'feat/lean-config' into main`. Merges the model-registry feature (above) with MCP client; alias resolution in `main.rs`/`agent.rs` via `Client::from_resolved`; TUI supports bare `/model` listing and Tab completion.

- **File-ref / skill / UI / MCP integration merge** — `a16a6d4` (2026-09-09) — `Merge branch 'feat/lean-ref-file' into main`. Integrates `@file` references, `$skill` forcing, boxed UI + queued approvals, and MCP client overlay. Resolves 4 `tui` conflicts (autocomplete alias, `draw_approval` MCP+queue, `/help` combine, `/model` live-switch) and dedupes `Msg` Clone impl.

- **Documentation** — `README.md` updated for MCP client (config, lifecycle, `/mcp` overlay, OAuth note).

- **Dependencies** — `Cargo.toml`/`Cargo.lock`: added `rmcp 3.2` (`client`, `transport-child-process`, `transport-streamable-http-client-reqwest`) and `http 1`.

## [0.1.0] - 2026-09-09

Initial release. Light, fast autonomous coding assistant — single native binary, TUI-first.

- Single binary (~7 MB), TUI with chat history + tool trace + streaming output (2000 char cap), multiline input, approval/session/allowlist overlays
- Agent loop (up to 100 steps, SSE streaming, tool routing, memory + skills), focus injection, `looks_complete` heuristic, `MAX_NOCALL_STREAK=3`
- Guards: bash allowlist (`wildmatch`) + CWD confinement (`dir_guard`) with approval UI
- Sessions: per-CWD persisted to `~/.lean/sessions/*.json` (pruned to 50), resume with `--continue`/`--resume`, Tab toggle
- Memory: `remember`/`search_memory`/`consolidate_memory` (Jaccard >0.75 dedup), autorecall
- Skills: `SKILL.md` discovery (`skills/`, `.lean/skills/`, `~/.agents/skills`), 60s cache, `read_skill` tool
- Tools: `read_file` (2000 lines/50 KB cap), `write_file`, `edit_file`, `bash` (50 KB tail), `web_search` (Exa → DuckDuckGo), memory tools
- Config: CLI flags > env vars > `.env` > defaults; `OPENCODE_*`/`OPENAI_*`/`EXA_*` via `dotenvy`

[Unreleased]: https://github.com/naet/lean/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/naet/lean/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/naet/lean/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/naet/lean/releases/tag/v0.1.0
