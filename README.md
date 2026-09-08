# lean — Rust port

Light coding assistant — autonomous agent with tools, now as a single native binary.

## Install & Run

```bash
cargo build --release
./target/release/lean --model mimo-v2.5-free
```

Or for development:

```bash
cargo run -- --model mimo-v2.5-free
cargo run -- --help
```

## TUI Layout

- **List** — chat history + tool calls + output (2000 char cap, truncated note shown)
- **Input** — multiline (Shift+Enter newline, Enter send), history Up/Down, `/help /model /clear /exit`, Esc quit, PgUp/PgDn Home/End scroll
- **Footer** — model | directory (CWD) | context stats (API usage + char fallback)

Single interactive mode only (one-shot and readline fallback removed).

## Configuration

Env vars (via `dotenvy` + `std::env`):

- `OPENCODE_API_KEY` or `OPENAI_API_KEY` (default `sk-test`)
- `OPENCODE_BASE_URL` (default `http://127.0.0.1:8080/zen/v1`)
- `EXA_API_KEY` (optional, for `web_search`; falls back to DuckDuckGo)

Model default: `mimo-v2.5-free`

## Tools (6)

`read_file`, `write_file`, `edit_file` (unique match), `bash` (`bash -c`), `web_search` (Exa→DuckDuckGo), `read_skill`

Truncation: 2000 lines / 50KB (head for files, tail for bash), 2KB for LLM.

## Skills

Scans `skills/`, `.lean/skills/`, `~/.agents/skills` for `SKILL.md` (frontmatter), 60s cache, `using-superpowers` filtered, loaded via `read_skill`.

## Development

```bash
cargo check
cargo build
```

No tests/CI for v1 (minimal).
