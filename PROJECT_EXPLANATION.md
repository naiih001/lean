# Project Explanation: Lean Coding Assistant (Rust Port)

## Overview
This is a Rust port of a "lean" coding assistant — an autonomous agent with tools, now as a single native binary. It's designed to be a lightweight, terminal-based AI assistant for coding tasks.

## Core Architecture
The project follows a modular architecture with these key components:

### 1. **Main Entry Point** (`src/main.rs`)
- Uses `clap` for CLI argument parsing
- Initializes the TUI (Terminal User Interface)
- Configures model selection (default: `mimo-v2.5-free`)

### 2. **Agent System** (`src/agent.rs`)
- Core autonomous agent loop with tool calling
- Manages conversation history and tool execution
- Implements streaming responses from LLM
- Handles tool result truncation (2KB for LLM, 2000 lines/50KB for TUI)

### 3. **LLM Client** (`src/llm.rs`)
- OpenAI-compatible API client
- Supports streaming completions via `reqwest` and `eventsource-stream`
- Environment-based configuration (`OPENCODE_API_KEY`, `OPENCODE_BASE_URL`)

### 4. **Tools** (`src/tools.rs`)
Six built-in tools:
- `read_file` - Read files from disk
- `write_file` - Write files (creates parent directories)
- `edit_file` - Edit files with unique text matching
- `bash` - Execute bash commands
- `web_search` - Search via Exa API or DuckDuckGo fallback
- `read_skill` - Load skill definitions from markdown files

### 5. **Skills System** (`src/skills.rs`)
- Scans `skills/`, `.lean/skills/`, and `~/.agents/skills` for `SKILL.md` files
- Parses YAML frontmatter for skill metadata
- Caches skills for 60 seconds
- Provides skill catalog to the agent's system prompt

### 6. **TUI** (`src/tui/`)
- Built with `ratatui` and `crossterm`
- Features:
  - Chat history display with tool call visualization
  - Multiline input area
  - Model and context information footer
  - Keyboard navigation (Up/Down history, Esc quit, PgUp/PgDn scroll)
  - Commands: `/help`, `/model`, `/clear`, `/exit`

### 7. **Theme** (`src/theme.rs`)
- Custom color palette ("Ashen") for the TUI
- Styles for different message types and UI elements

## Key Features
1. **Autonomous Operation**: Agent can plan and execute multi-step tasks
2. **Tool Integration**: Seamless tool calling with real-time streaming
3. **Skill Extensibility**: Load custom skills from markdown files
4. **Terminal Interface**: Full-featured TUI with syntax highlighting
5. **Configuration**: Environment variable based with sensible defaults

## Configuration
- `OPENCODE_API_KEY` or `OPENAI_API_KEY` (default: `sk-test`)
- `OPENCODE_BASE_URL` (default: `http://127.0.0.1:8080/zen/v1`)
- `EXA_API_KEY` (optional for web search)

## Development
- Built with Tokio for async runtime
- Uses `ratatui` for terminal UI
- Minimal dependencies (no tests/CI for v1)

## Usage
```bash
# Build and run
cargo build --release
./target/release/lean --model mimo-v2.5-free

# Development
cargo run -- --model mimo-v2.5-free
```

The project represents a clean, focused implementation of an AI coding assistant with a strong emphasis on terminal usability and extensibility through skills.