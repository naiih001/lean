# MEMORY.md — Who I Am (User Persona)

> This file is loaded on every lean session to personalize assistance. 3-5 short sections is ideal — concise but enough for the agent to understand your perspective. Edit freely; the agent will respect it.

## About Me
- **Name / handle:** naet
- **Role:** Senior backend engineer
- **Location / timezone:** Lagos, Africa (WAT, UTC+1)
- **Experience:** Backend engineering, Rust/TUI development (building lean)

## Preferences
- **Communication style:** Direct and concise — short, no fluff, code over prose
- **Code style:** Idiomatic Rust, explicit error handling, keep diffs minimal
- **Tools:** [fill in: editor, terminal, OS]
- **Language:** English

## Goals
- **Current focus:** Building lean — a light, fast autonomous coding assistant
- **Long-term:** [fill in]

## Working Style
- **How I like to work:** Bias to doing, plan first then build
- **When to ask vs. act:** Ask for ambiguous scope, otherwise proceed
- **Constraints:** Avoid over-engineering, keep diffs minimal

## Context the Agent Should Remember
- lean is a single-binary Rust TUI coding assistant with SSE streaming, MCP support, and tool guards
- Sessions are per-CWD in ~/.lean/sessions
- Requires OPENAI_API_KEY or OPENCODE_API_KEY in .env

---
*Tip: run `/init` to regenerate AGENT.md from the codebase; edit this file directly to refine how lean understands you.*
