---
name: lean-config
description: Manage lean's own configuration — env (.env), LLM provider (API keys, base URL, model), guard allowlists, and persistent storage (~/.lean/). Use when user asks to view, change, or validate lean configs, switch models, set keys, toggle guards, or inspect/clean sessions and memory.
---

# Lean Config — Self-Configuration Skill

Lean is the lightweight terminal AI assistant in this repo (`/home/naet/Documents/lean`). This skill lets it inspect and safely mutate its *own* configs without human manual edits.

## When to Use

- User says: "change model", "set API key", "use OpenAI", "point to localhost:8080", "disable guard", "allow this command", "show my config", "where is my memory/sessions", "clean old sessions", "config is broken"
- On task start if you need to know which model/provider you’re running with
- Before writing outside CWD or running a risky bash command — check guard allowlists first

## Config Inventory (Source of Truth)

Read these with `read_file`. Never assume — always read before editing.

| Config | Path | Format | What it controls |
|---|---|---|---|
| **Env file** | `./.env` (project root) + any `dotenvy::dotenv()` loads | `KEY=VALUE` lines | `OPENCODE_API_KEY`, `OPENAI_API_KEY`, `OPENCODE_BASE_URL`, `EXA_API_KEY`, `LEAN_BASH_GUARD_DISABLED`, `LEAN_DIR_GUARD_DISABLED`, `LEAN_OBSERVER_DISABLED` |
| **CLI args** | `src/main.rs` `Args` | `clap` | `--model` (default `mimo-v2.5-free`), `--continue`, `--resume <id>`, `--no-session`, `--bash-guard-disabled`, `--dir-guard-disabled` |
| **LLM client** | `src/llm.rs` `Client::from_env()` | env var precedence | `OPENCODE_API_KEY` > `OPENAI_API_KEY` > `sk-test`; `OPENCODE_BASE_URL` > `http://127.0.0.1:8080/zen/v1` |
| **Bash guard allowlist** | `~/.lean/allowlist.json` | `JSON array<string>` | Exact commands or `wildmatch` globs (`git status*`, `cargo check*`). Loaded in `src/bash_guard.rs` |
| **Dir guard allowlist** | `~/.lean/dir_allowlist.json` | `JSON array<string>` | Paths/globs allowed outside CWD (`~/docs/*`, `/tmp/*`). Loaded in `src/dir_guard.rs` |
| **Memory** | `~/.lean/memory.json` | `JSON array<MemoryEntry>` | Persistent memories (`src/memory.rs`, max 500, dedup) |
| **Sessions** | `~/.lean/sessions/*.json` | `JSON Session` | Per-model/CWD chat history + `llm_history` (pruned to 50, `src/session.rs`) |
| **Project root** | `std::env::current_dir()` at `tui::run` + `dir_guard::init()` | — | CWD shown in footer; hard wall for `analyze_path`/`analyze_bash` |
| **Telemetry** | `dirs::config_dir()/lean` or `~/.config/lean` | — | `src/telemetry.rs` (optional) |

**Precedence (highest first):** CLI flag > env var > `.env` file > hardcoded default.

## Workflow

### 1. View current effective config

1. `read_file` on `./.env` (if exists), `~/.lean/allowlist.json`, `~/.lean/dir_allowlist.json`
2. `bash` with `env | grep -E 'OPENCODE|OPENAI|EXA_|LEAN_' | sort` to see live env (redact values: show `sk-...XXXX` not full key)
3. Check `src/main.rs` defaults, `src/llm.rs` `DEFAULT_MODEL`, `Cargo.toml` version
4. Summarize: `model=...`, `base_url=...`, `api_key= set|missing (source)`, `bash_guard= on|off`, `dir_guard= on|off`, `allowlist counts`, `sessions count`, `memory count`

Never print a full API key. Show `OPENCODE_API_KEY=sk-...abcd (from .env)` or `OPENAI_API_KEY= unset`.

### 2. Change model

User: "/model gpt-4o" or "use mimo-v2-flash" or "switch to local model"

- For **current session only**: no file write — just report `model: <name> (restart to apply)` as `tui/mod.rs` does. Tell user `cargo run -- --model <name>` or `lean --model <name>`.
- For **persistent default**: `read_file` `src/main.rs` line `#[arg(long, default_value = "...")]` and `read_file` `./.env`. Prefer `.env` override: `LEAN_MODEL` is *not* standard — so edit `src/main.rs` default OR add `MODEL=...` and update `main.rs` to read it. Simplest: edit `src/main.rs` default_value and run `cargo check` to verify.
- Validate: after edit run `bash` `cargo check` and confirm `--help` shows new default.

### 3. Set provider / API key / base URL

User: "use OpenAI", "set my Exa key", "point to http://localhost:11434/v1"

1. `read_file` `.env` (create if missing). `bash` `ls -la .env*` to confirm.
2. Decide var: `OPENCODE_API_KEY` for zen proxy, `OPENAI_API_KEY` for direct OpenAI, `OPENCODE_BASE_URL`, `EXA_API_KEY`.
3. **Write safely** — never `edit_file` with non-unique blurb. For `.env`:
   - If key exists: `read_file` → `edit_file` with unique oldText `KEY=old` → `KEY=new`
   - If missing: `read_file` → `write_file` appending `\nKEY=VALUE\n` (use `bash` `cat .env` or read then write full content; ensure not truncating)
   - Alternative robust: `bash` with `grep -q '^KEY=' .env && sed -i 's/^KEY=.*/KEY=new/' .env || echo 'KEY=new' >> .env` — prefer file tools but `bash` append is acceptable for dotfiles.
4. Validate: `bash` `grep -v '^#' .env | grep -E 'OPENCODE|OPENAI|EXA_'` (redact) and `bash` `cargo check` if you touched `src/llm.rs`.
5. Security: `bash` `chmod 600 .env` if it contains keys, ensure ` .env` is in `.gitignore` (`read_file` `.gitignore` → add if missing).

Example `.env`:
```
OPENCODE_API_KEY=sk-xxxx
OPENCODE_BASE_URL=http://127.0.0.1:8080/zen/v1
EXA_API_KEY=...
# LEAN_BASH_GUARD_DISABLED=1  # uncomment to disable
```

### 4. Toggle or configure guards

- **Disable once (session)**: `--bash-guard-disabled` / `--dir-guard-disabled` CLI, or env `LEAN_BASH_GUARD_DISABLED=1`, `LEAN_DIR_GUARD_DISABLED=1` in `.env`.
- **Persistent allowlist** (preferred over disable): 
  - Bash: `~/.lean/allowlist.json` — `read_file` → `edit_file` or `bash` `cat ~/.lean/allowlist.json | jq .` . Add via `bash` `jq` or file tools: ensure JSON array stays valid.
  - Dir: `~/.lean/dir_allowlist.json` — same. Supports `~/`, `*`, prefix match.
  - In-TUI: `/allowlist`, `/allowlist add <pattern>`, `/allowlist clear`, `Tab` in picker — tell user.
- After editing allowlist, `bash` `cat ~/.lean/allowlist.json` and `cat ~/.lean/dir_allowlist.json` to confirm, then test with a harmless `bash` that previously was blocked.

### 5. Manage sessions & memory

- List: `bash` `ls -lh ~/.lean/sessions/ | tail -n 20` and `read_file` a sample `~/.lean/sessions/<id>.json` (truncate preview).
- Count/clean: `src/session.rs` `prune(50)` auto-prunes. To manually clean: `bash` `ls ~/.lean/sessions/*.json | wc -l` then `bash` `rm ~/.lean/sessions/<old>.json` only if user asked and confirm age via `stat`.
- Memory: `read_file` `~/.lean/memory.json` (large — preview first 50 lines via `bash` `head -n 50 ~/.lean/memory.json`). Use tools `search_memory`, `recall_memory`, `forget_memory`, `consolidate_memory` via agent tools, not raw edit unless corrupted. If corrupted JSON, backup then `write_file` fixed JSON.

### 6. Validate after any change

Always:

```bash
cargo check
cat .env | sed 's/=.*/=***redacted***/'  # show without leaking
cat ~/.lean/allowlist.json | head -n 50
ls -lh ~/.lean/sessions/ | head
```

If `cargo check` fails, revert the edit (`edit_file` back or restore from `read_file` backup you kept in context).

## Safety Rules

- **Never echo full keys** in tool output, commit messages, or `memory.json`. Redact to `sk-...last4`.
- **Backup before overwrite**: `bash` `cp .env .env.bak 2>/dev/null; cp ~/.lean/allowlist.json ~/.lean/allowlist.json.bak 2>/dev/null` when you’re about to mutate.
- **Keep `.env` gitignored**: if `read_file` `.gitignore` lacks `.env`, append it.
- **Use truncation-aware tools**: `read_file` truncates at 2000 lines/50KB head; for large `memory.json` use `bash` `wc -l`/`head`/`jq` to avoid truncation loss.
- **Prefer allowlist over disabling guards**. Only set `LEAN_*_DISABLED=1` if user explicitly says "disable guard".
- **Don’t create `~/.lean/config.json` unless user asks** — lean currently has no central config file; env + allowlists are the config. If you do create one, document it and make `src/main.rs`/`llm.rs` read it, then `cargo check`.

## Examples

**User: "show my lean config"**
→ Read `.env`, `~/.lean/allowlist.json`, `~/.lean/dir_allowlist.json`, `env | grep ...` (redacted), count sessions/memory, report model/base_url/guard states with file paths and redacted values.

**User: "switch to gpt-4o-mini and use OpenAI directly"**
→ `read_file` `.env` → `edit_file`/`write_file` to set `OPENAI_API_KEY=sk-...` and `OPENCODE_BASE_URL=https://api.openai.com/v1` (or unset to use default proxy) → `bash` `cargo check` → instruct `cargo run -- --model gpt-4o-mini`.

**User: "allow git push --force for this repo"**
→ `read_file` `~/.lean/allowlist.json` → add `"git push --force"` or `"git push*"` → `write_file` valid JSON array → verify `cat`.

**User: "lean keeps blocking /tmp writes, allow it"**
→ `read_file` `~/.lean/dir_allowlist.json` → add `"/tmp/*"` → write → `bash` `cat ~/.lean/dir_allowlist.json` → test `bash` `touch /tmp/lean_test_ok && rm /tmp/lean_test_ok`.

**User: "my .env is broken, fix it"**
→ `read_file` `.env` → parse lines, report `KEY=...` with redacted values, fix duplicate keys, missing newline, quoting → `write_file` corrected → `bash` `cargo check`.

## Verification Checklist

- [ ] Read before write — you called `read_file` on every file you edited
- [ ] Redacted secrets in any summary
- [ ] `cargo check` passes after code changes
- [ ] `~/.lean/*.json` remains valid JSON (`bash` `jq empty <file>` or `python3 -m json.tool`)
- [ ] `.env` still `600` and gitignored
- [ ] Reported effective config (model, base_url, guard states) and what changed
