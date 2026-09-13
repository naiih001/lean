use std::path::{Path, PathBuf};

const MAX_FILE_CHARS: usize = 8000;
const MAX_TOTAL_CHARS: usize = 24000;

/// All context filenames we support.
/// Project: only AGENT.md (created by /init).
/// Global (background): AGENT.md + MEMORY.md (MEMORY.md auto-updated silently in ~/.lean/).
const FILENAMES: &[&str] = &["AGENT.md"];

fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

fn project_root() -> PathBuf {
    crate::dir_guard::project_root()
}

/// All candidate paths in priority order: global first, then project.
/// Duplicates are deduped by canonical check at load time.
/// Project only loads AGENT.md; MEMORY.md is global-only (background, ~/.lean/).
fn candidate_paths() -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    let home = home_dir();
    let cwd = project_root();

    // Global candidates — low priority, shown first.
    if let Some(home) = &home {
        // ~/.lean/AGENT.md + MEMORY.md (global config, MEMORY.md updated silently in background)
        for name in FILENAMES {
            out.push((home.join(".lean").join(name), "global"));
        }
        out.push((home.join(".lean").join("MEMORY.md"), "global"));
        // ~/.claude compat (legacy)
        out.push((home.join(".claude").join("MEMORY.md"), "global"));
        // ~/AGENT.md (direct home)
        for name in FILENAMES {
            out.push((home.join(name), "global"));
        }
        out.push((home.join("MEMORY.md"), "global"));
    }

    // Project candidates — higher priority. Only AGENT.md; MEMORY.md is global-only.
    for name in FILENAMES {
        out.push((cwd.join(name), "project"));
    }
    // ./.lean/AGENT.md (project-local lean config)
    for name in FILENAMES {
        out.push((cwd.join(".lean").join(name), "project"));
    }

    out
}

/// Return list of existing context files with their scope tag.
pub fn existing_files() -> Vec<(PathBuf, String)> {
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut found = Vec::new();
    for (path, scope) in candidate_paths() {
        // Deduplicate same resolved path (e.g., symlink)
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        if seen.contains(&canon) {
            continue;
        }
        if path.is_file() {
            seen.insert(canon);
            found.push((path, scope.to_string()));
        }
    }
    found
}

/// Read and format all context files into a single markdown section.
/// Returns None if no files exist.
pub fn load_context_section() -> Option<String> {
    let files = existing_files();
    if files.is_empty() {
        return None;
    }

    let mut sections: Vec<String> = Vec::new();
    let mut total = 0usize;
    // Track content hashes to avoid duplicating identical content copies.
    let mut seen_hashes: std::collections::HashSet<u64> = std::collections::HashSet::new();

    for (path, scope) in files {
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Hash for dedup (normalize whitespace)
        let hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            trimmed.hash(&mut h);
            h.finish()
        };
        if seen_hashes.contains(&hash) {
            continue;
        }
        seen_hashes.insert(hash);

        // Per-file truncation
        let content = if trimmed.chars().count() > MAX_FILE_CHARS {
            let truncated: String = trimmed.chars().take(MAX_FILE_CHARS).collect();
            format!(
                "{}…\n[truncated: file was {} chars, showing first {}]",
                truncated,
                trimmed.chars().count(),
                MAX_FILE_CHARS
            )
        } else {
            trimmed.to_string()
        };

        // Pretty display path
        let display = display_path(&path, &scope);
        let label = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("CONTEXT.md");
        let header = format!("### {} ({}: {})", label, scope, display);
        let section = format!("{}\n{}", header, content);
        total += section.chars().count();
        if total > MAX_TOTAL_CHARS {
            let remaining = MAX_TOTAL_CHARS.saturating_sub(total - section.chars().count());
            if remaining < 500 {
                sections
                    .push("[additional context files omitted — total budget exceeded]".to_string());
                break;
            } else {
                let truncated: String = section.chars().take(remaining).collect();
                sections.push(format!(
                    "{}…\n[truncated: total context budget {} chars exceeded]",
                    truncated, MAX_TOTAL_CHARS
                ));
                break;
            }
        }
        sections.push(section);
    }

    if sections.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str("## Project & User Context (AGENT.md + global MEMORY.md)\n");
    out.push_str("The following files were loaded from disk — treat them as high-priority persistent context. Project AGENT.md overrides global ones when they conflict. Global MEMORY.md (~/.lean/MEMORY.md) is updated silently in the background. Follow their instructions, conventions, and preferences.\n\n");
    out.push_str(&sections.join("\n\n---\n\n"));
    Some(out)
}

fn display_path(path: &Path, scope: &str) -> String {
    if scope == "global" {
        if let Some(home) = home_dir() {
            if let Ok(rel) = path.strip_prefix(&home) {
                return format!("~/{}", rel.display());
            }
        }
        path.display().to_string()
    } else {
        let root = project_root();
        if let Ok(rel) = path.strip_prefix(&root) {
            if rel.as_os_str().is_empty() {
                return ".".to_string();
            }
            return format!("./{}", rel.display());
        }
        path.display().to_string()
    }
}

/// Short one-line summary of loaded context files for the startup banner.
pub fn load_summary() -> Option<String> {
    let files = existing_files();
    if files.is_empty() {
        return None;
    }
    let names: Vec<String> = files
        .iter()
        .map(|(p, scope)| {
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            format!("{} ({})", fname, scope)
        })
        .collect();
    Some(names.join(", "))
}

// ── Init templates ───────────────────────────────────────────────

pub fn agent_template(project_name: &str) -> String {
    format!(
        r#"# AGENT.md — Project Context for lean

> This file is loaded on every lean session. Keep it concise and actionable.

## Project Overview
- **Name:** {project_name}
- **Purpose:** [1-2 sentences: what does this project do? Who is it for?]
- **Status:** [active / prototype / maintained]

## Tech Stack
- **Language(s):** [e.g., Rust, TypeScript]
- **Framework(s):** [e.g., Ratatui, Next.js]
- **Package manager / build:** [e.g., cargo, npm, pnpm]
- **Key dependencies:** [list 3-6 important libs]

## Commands
```bash
# Build
cargo build --release        # or npm run build

# Run
cargo run                    # or npm run dev

# Test
cargo test                   # or npm test
cargo check                  # fast type-check without running

# Lint / Format
cargo fmt --check
# Add other project-specific commands below
```

## Project Structure
```
.
├── src/          # source code
├── skills/       # lean skills (SKILL.md per folder)
├── .lean/        # lean sessions, plans, local config
├── tests/        # tests
└── README.md
```
- **Entry points:** [e.g., src/main.rs, src/lib.rs]
- **Important modules:** [briefly note key files and their roles]

## Conventions
- **Style:** [e.g., `cargo fmt`, single binary, no extra runtime deps]
- **Commits:** [e.g., conventional commits, brief messages]
- **Branching:** [e.g., main + feature branches]
- **Do / Don't:** [e.g., "Read before edit; smallest change that solves the problem; verify once with cargo check"]

## Architecture Notes
- [Key patterns, constraints, or design decisions the agent must respect]
- [E.g., "TUI → agent loop → tools/MCP/LLM", "sessions are per-CWD in ~/.lean/sessions"]

## Gotchas
- [Common pitfalls, env vars needed, secrets handling]
- [E.g., Requires `OPENAI_API_KEY` or `OPENCODE_API_KEY`, `libssl3` on Linux]
"#
    )
}

#[allow(dead_code)]
pub fn claude_mirror_note() -> String {
    "This file mirrors AGENT.md for Claude Code compatibility. Keep them in sync (or symlink CLAUDE.md → AGENT.md).".to_string()
}

#[allow(dead_code)]
pub fn memory_template() -> String {
    r#"# MEMORY.md — Who I Am (User Persona)

> This file is loaded on every lean session to personalize assistance. 3-5 short sections is ideal — concise but enough for the agent to understand your perspective. Edit freely; the agent will respect it.

## About Me
- **Name / handle:** [your name]
- **Role:** [e.g., indie hacker, senior backend engineer, student]
- **Location / timezone:** [e.g., UTC+8, Europe/Berlin]
- **Experience:** [1-2 lines about your background]

## Preferences
- **Communication style:** [e.g., direct and concise, thorough with examples, prefer code over prose]
- **Code style:** [e.g., idiomatic Rust, functional preference, explicit error handling]
- **Tools:** [e.g., Neovim, VS Code, Ghostty, Linux]
- **Language:** [e.g., English, or mix]

## Goals
- **Current focus:** [what you're building or learning right now]
- **Long-term:** [bigger direction, if you want the agent to know]

## Working Style
- **How I like to work:** [e.g., plan first then build, bias to doing, ask before big changes]
- **When to ask vs. act:** [e.g., ask for ambiguous scope, otherwise proceed]
- **Constraints:** [e.g., avoid over-engineering, keep diffs minimal]

## Context the Agent Should Remember
- [Anything else: past decisions, product context, team notes — keep to a few paragraphs]

---
*Tip: run `/init` to regenerate AGENT.md from the codebase; edit this file directly to refine how lean understands you.*
"#.to_string()
}

/// Ensure init files exist, creating them with templates if missing.
/// Returns list of created/updated paths for display.
/// Only creates ./AGENT.md in the project — MEMORY.md is global-only
/// and updated silently in the background (~/.lean/MEMORY.md).
/// This is the deterministic fallback used when the LLM is unavailable;
/// the primary `/init` flow spawns an agent to generate richer content.
pub fn ensure_init_files() -> Vec<String> {
    let cwd = project_root();
    let project_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let mut created = Vec::new();

    // AGENT.md — project only
    let agent_path = cwd.join("AGENT.md");
    if !agent_path.exists() {
        let content = agent_template(project_name);
        if let Some(parent) = agent_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&agent_path, content).is_ok() {
            created.push(agent_path.display().to_string());
        }
    }

    created
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_non_empty() {
        assert!(agent_template("test").contains("AGENT.md"));
        assert!(memory_template().contains("MEMORY.md"));
    }

    #[test]
    fn display_path_project() {
        // just ensure it doesn't panic
        let p = PathBuf::from("/tmp/foo/AGENT.md");
        let s = display_path(&p, "project");
        assert!(!s.is_empty());
    }
}
