# Lean → Hermes-Class Agent: Gap Analysis and Roadmap

> Lean is a solid Rust TUI coding assistant. This plan maps what it takes to become a Hermes Agent alternative — a persistent, multi-platform, self-evolving autonomous agent.

---

## 1. Where Lean Stands Today

| Capability | Status | Notes |
|---|---|---|
| TUI (terminal chat) | Done | Ratatui, streaming, tool trace, multiline input |
| Agent loop | Done | SSE streaming, tool-call routing, 100 steps/turn |
| File tools | Done | read, write, edit, grep, find, ls |
| Bash execution | Done | With allowlist guard, approval UI, sudo injection |
| Web search | Done | DuckDuckGo + Exa fallback |
| Web fetch | Done | URL fetch with markdown/text extraction |
| Memory | Done | JSON-based, keyword search, dedup, recall |
| Skills | Done | SKILL.md discovery from project/global dirs |
| MCP client | Done | rmcp with stdio + HTTP transports |
| Sessions | Done | Per-CWD persistence, resume |
| Modes | Done | Norm, Plan, Ask, Auto |
| Providers | Done | OpenAI, Anthropic, Ollama, Generic |
| Image reading | Done | Base64 multimodal input |
| Diff view | Done | Unified diff on file edits |
| Guards | Done | Bash allowlist, directory confinement |

**Lean is a strong coding assistant. The gaps below are what separate it from an autonomous agent platform.**

---

## 2. What Hermes Agent Has That Lean Does Not

### Tier 1 — Core Agent Gaps (Must-Have)

| Feature | Hermes Has | Lean Has | Gap |
|---|---|---|---|
| **Persistent cross-session memory** | SQLite + FTS5 search, full conversation history | JSON file, keyword match, 500 entries | Lean's memory is primitive — no full-text search, no conversation store |
| **Learning loop** | Auto-creates skills after 3+ task repetitions, refines from feedback | Static SKILL.md files only | No self-improvement mechanism |
| **Scheduled execution (cron)** | Natural language scheduling, background jobs, cron expressions | None | No way to run tasks without user present |
| **Multi-platform gateway** | Telegram, Discord, Slack, WhatsApp, Signal, Email, GitHub | Terminal only | Single interface |
| **Sandboxed code execution** | Python sandbox with 5 backends (local, Docker, SSH, Singularity, Modal) | Raw bash | No isolation, no safe code execution |
| **Subagent delegation** | Isolated subagents with own context, terminals, Python RPC | None | No parallel task execution |

### Tier 2 — Important Gaps

| Feature | Hermes Has | Lean Has | Gap |
|---|---|---|---|
| **Browser automation** | Browser Use CLI 3.0, vision, web interaction | Web fetch only (no JS, no interaction) | Can't fill forms, click, navigate |
| **Image generation** | Built-in | None | Not applicable to coding? (low priority) |
| **Text-to-speech** | Built-in | None | Not critical for coding |
| **Desktop app** | Electron/Tauri for macOS + Windows | Terminal only | Limits non-CLI users |
| **Dashboard** | Web UI for memory, skills, cron, gateway management | None | No remote management |
| **Cloud deployment** | Hosted option, Nous Portal integration | None | No SaaS path |
| **Model routing** | 200+ models via Nous Portal, automatic fallback | Manual model config | No smart routing |

### Tier 3 — Nice-to-Have

| Feature | Hermes Has | Lean Has | Gap |
|---|---|---|---|
| **Docker containerization** | First-class Docker support | None | Harder to deploy |
| **GitHub integration** | Issue, PR, Actions workflow support | None | No CI/CD automation |
| **Migration tools** | Import from OpenClaw etc. | None | N/A unless competing |
| **Community skills marketplace** | 80+ community projects | None | No ecosystem |

---

## 3. Rust Advantages (Why This Can Win)

Hermes Agent is **Python**. Lean is **Rust**. This is a genuine edge:

- **Performance**: Single binary, sub-100ms startup vs Python's cold start
- **Memory**: 10-50x less RAM usage
- **Safety**: Memory safety without GC, type-safe state management
- **Distribution**: Single binary, no runtime deps
- **Concurrency**: Tokio async is more efficient than Python asyncio

**The pitch**: "All the power of Hermes, in a single binary that starts instantly and uses 20MB of RAM."

---

## 4. Phased Roadmap

### Phase 1: Memory & Learning Foundation (Weeks 1-3)

**Goal**: Close the memory gap — this is the #1 differentiator.

```
Priority: CRITICAL
Effort: 2-3 weeks
Impact: Transforms lean from stateless tool to persistent agent
```

1. **SQLite memory store** (replace `memory.json`)
   - SQLite for structured storage (memories, conversations, facts)
   - FTS5 for full-text search across all memories
   - Schema: `memories`, `conversations`, `facts`, `skills` tables
   - Migration tool from existing `memory.json`

2. **Conversation persistence**
   - Store full conversation history in SQLite (not just 50-message trim)
   - Indexed by session, timestamp, and topic
   - Support for cross-session context queries

3. **Auto-skill creation** (the learning loop)
   - Track repeated tool-call patterns (same sequence 3+ times)
   - Propose converting pattern into a SKILL.md
   - Allow user to approve/reject skill creation
   - Skill refinement from execution feedback

4. **Enhanced memory tools**
   - `search_memory` with FTS5 ranking (replace keyword match)
   - `forget_memory` with soft-delete (recoverable)
   - `consolidate_memory` with semantic similarity
   - `export_memory` / `import_memory` for portability

### Phase 2: Scheduling & Background Execution (Weeks 4-6)

**Goal**: Lean can work without the user watching.

```
Priority: HIGH
Effort: 2-3 weeks
Impact: Enables autonomous operation
```

1. **Cron scheduler**
   - Natural language → cron expression parsing
   - Background task runner with SQLite-backed job store
   - Support: one-shot, interval, cron-expression schedules
   - Persistent across restarts

2. **Background agent runner**
   - Spawn isolated agent sessions for scheduled tasks
   - Capture output to session logs
   - Notification on completion (terminal bell, optional webhook)

3. **Job management UI**
   - `/cron` slash command to list, add, remove, inspect jobs
   - TUI overlay showing active scheduled jobs
   - Status: running, completed, failed, next-run

### Phase 3: Multi-Platform Gateway (Weeks 7-12)

**Goal**: Lean works everywhere, not just the terminal.

```
Priority: HIGH
Effort: 5-6 weeks
Impact: Massively expands use cases
```

1. **Gateway architecture**
   - Single gateway process that routes messages to lean agent
   - Plugin system for platform adapters
   - Shared memory and skills across all platforms

2. **Platform adapters** (ordered by priority)
   - **Telegram** (week 7-8): Bot API, inline keyboard, media support
   - **Discord** (week 9): Bot with slash commands, thread support
   - **GitHub** (week 10): Issue/PR integration, Actions webhook
   - **Slack** (week 11): Workspace bot, channel/DM support
   - **Webhook/Email** (week 12): Generic webhook receiver, email polling

3. **Unified context**
   - Same memory, skills, and session state across all platforms
   - Platform-aware formatting (Markdown for Telegram, embeds for Discord)
   - Cross-platform conversation threading

### Phase 4: Sandboxed Execution (Weeks 13-16)

**Goal**: Safe code execution without risking the host.

```
Priority: MEDIUM-HIGH
Effort: 3-4 weeks
Impact: Enables safe experimentation and code execution
```

1. **Execution backends**
   - **Docker** (primary): Container with mounted workspace, resource limits
   - **Local process**: nsjail or bubblewrap for namespace isolation
   - **SSH**: Remote execution on specified hosts

2. **Python execution environment**
   - Pre-built Docker image with common libraries
   - File mounting for project access
   - Timeout and resource limits

3. **Subagent system**
   - Spawn isolated agent sessions with their own context
   - Communicate via message passing
   - Parent agent orchestrates, children execute

### Phase 5: Browser & Web Automation (Weeks 17-20)

**Goal**: Lean can interact with the web like a human.

```
Priority: MEDIUM
Effort: 3-4 weeks
Impact: Enables research, scraping, form filling
```

1. **Browser integration**
   - Integrate `headless-chrome` or `playwright` crate
   - Tool: `browser_navigate`, `browser_click`, `browser_type`, `browser_screenshot`
   - Session management (cookies, auth state)

2. **Vision capabilities**
   - Screenshot analysis via multimodal LLM
   - DOM extraction for structured data
   - PDF/document rendering and reading

3. **Web automation workflows**
   - Combine browser + web search for research workflows
   - Form filling for automated data entry
   - Monitoring: watch pages for changes

### Phase 6: Model Intelligence & Routing (Weeks 21-24)

**Goal**: Smart model selection and fallback.

```
Priority: MEDIUM
Effort: 2-3 weeks
Impact: Better cost/quality tradeoffs
```

1. **Model router**
   - Task classification → model selection (coding task? use Claude; quick question? use small model)
   - Automatic fallback on failure/timeout
   - Cost tracking per model

2. **Provider abstraction**
   - Unified provider interface
   - Support for: OpenAI, Anthropic, Google Gemini, Ollama, Together, Groq, Mistral
   - Nous Portal integration (optional)

3. **Local model support**
   - Ollama auto-detection and model management
   - GGUF model loading for fully offline operation
   - Model download and update commands

### Phase 7: Distribution & Packaging (Weeks 25-28)

**Goal**: Easy installation and deployment.

```
Priority: MEDIUM
Effort: 2-3 weeks
Impact: Adoption and ecosystem growth
```

1. **Docker image**
   - Multi-stage build: lean binary + gateway + optional browser
   - docker-compose for full stack (lean + SQLite + browser)
   - Health checks and graceful shutdown

2. **Desktop app**
   - Tauri (Rust-based, lighter than Electron)
   - System tray with quick-access chat
   - Auto-update mechanism

3. **Package managers**
   - Homebrew formula (macOS/Linux)
   - AUR package (Arch)
   - Snap/Flatpak (Linux)
   - winget (Windows)

4. **Skills marketplace**
   - `lean skill search <query>`
   - `lean skill install <name>`
   - Registry server for community skills

---

## 5. Quick Wins (Start Here)

These can ship in days, not weeks:

| Win | Effort | Impact |
|---|---|---|
| SQLite memory store (replaces JSON) | 3-4 days | Foundation for everything |
| Conversation persistence in SQLite | 2-3 days | Cross-session context |
| `/cron` basic scheduler (one-shot only) | 3-4 days | Background tasks |
| Docker image | 2-3 days | Easy deployment |
| Homebrew formula | 1 day | Easy installation |
| Telegram bot adapter | 4-5 days | First multi-platform |
| Auto-skill proposal (pattern detection) | 3-4 days | Learning loop v1 |

---

## 6. Architecture Diagram

```
┌─────────────────────────────────────────────────────┐
│                    Lean Agent Core                    │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────┐  │
│  │ Agent    │  │ Memory   │  │ Skills           │  │
│  │ Loop     │  │ (SQLite) │  │ (auto-create)    │  │
│  └────┬─────┘  └────┬─────┘  └────────┬─────────┘  │
│       │              │                  │             │
│  ┌────┴──────────────┴──────────────────┴─────┐     │
│  │           Tool Router                       │     │
│  │  files │ bash │ web │ browser │ cron │ mcp  │     │
│  └────┬──────┬──────┬──────┬──────┬──────┬────┘     │
│       │      │      │      │      │      │          │
│  ┌────┴──────┴──────┴──────┴──────┴──────┴────┐     │
│  │        Sandbox Layer                        │     │
│  │  Docker │ nsjail │ SSH │ Modal             │     │
│  └─────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────┘
                         │
                         │ Gateway
         ┌───────────────┼───────────────┐
         │               │               │
    ┌────┴────┐    ┌─────┴─────┐   ┌─────┴─────┐
    │Terminal │    │ Telegram  │   │ Discord   │
    │  (TUI)  │    │ Discord   │   │ Slack     │
    │         │    │ Slack     │   │ GitHub    │
    └─────────┘    │ WhatsApp  │   │ Webhook   │
                   │ Email     │   └───────────┘
                   └───────────┘
```

---

## 7. Dependencies to Add

| Crate | Purpose | Phase |
|---|---|---|
| `rusqlite` | SQLite memory store | 1 |
| `tokio-cron-scheduler` | Background job scheduling | 2 |
| `teloxide` | Telegram bot API | 3 |
| `serenity` / `poise` | Discord bot | 3 |
| `octocrab` | GitHub API | 3 |
| `bollard` | Docker API client | 4 |
| `headless-chrome` or `thirtyfour` | Browser automation | 5 |
| `tauri` | Desktop app | 7 |

---

## 8. Success Criteria

A "Hermes alternative" means lean can:

- [ ] Remember everything across sessions (SQLite + FTS5)
- [ ] Learn and create skills from repeated patterns
- [ ] Run scheduled tasks autonomously
- [ ] Work through Telegram/Discord/Slack/GitHub
- [ ] Execute code in sandboxed environments
- [ ] Delegate to subagents for parallel work
- [ ] Automate browser interactions
- [ ] Smart model selection and fallback
- [ ] Single binary distribution + Docker
- [ ] Community skills marketplace

**All while being written in Rust, distributed as a single binary, using 10-50x less RAM than the Python original.**

---

## 9. Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Scope creep | High | Strict phasing, ship incrementally |
| SQLite FTS5 complexity | Medium | Start simple, upgrade later |
| Platform API changes | Medium | Abstract adapters, test regularly |
| Browser automation fragility | Medium | Start with read-only, add interaction later |
| Community adoption | High | Ship Docker + Homebrew early, showcase Rust advantage |

---

*Last updated: 2026-09-14*
