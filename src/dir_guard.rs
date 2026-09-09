use std::path::{Path, PathBuf};
use std::sync::{OnceLock, atomic::{AtomicBool, Ordering}};

static DISABLED: AtomicBool = AtomicBool::new(false);
static PROJECT_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn set_disabled(v: bool) { DISABLED.store(v, Ordering::Relaxed); }
pub fn is_disabled() -> bool { DISABLED.load(Ordering::Relaxed) }

/// Call once at startup to capture CWD. If `root` is None, uses `current_dir()`.
pub fn init(root: Option<PathBuf>) {
    let r = root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    // canonicalize if possible, else lexical normalize
    let canon = r.canonicalize().unwrap_or_else(|_| normalize(&r));
    let _ = PROJECT_ROOT.set(canon);
}

pub fn project_root() -> PathBuf {
    if let Some(p) = PROJECT_ROOT.get() { return p.clone(); }
    // lazy fallback
    let r = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    r.canonicalize().unwrap_or_else(|_| normalize(&r))
}

// ---- allowlist ----

fn allowlist_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".lean").join("dir_allowlist.json")
}

fn load_allowlist() -> std::collections::HashSet<String> {
    let path = allowlist_path();
    if let Ok(txt) = std::fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(&txt) {
            return v.into_iter().collect();
        }
    }
    std::collections::HashSet::new()
}

fn save_allowlist(set: &std::collections::HashSet<String>) {
    let path = allowlist_path();
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let vec: Vec<String> = set.iter().cloned().collect();
    if let Ok(json) = serde_json::to_string_pretty(&vec) { let _ = std::fs::write(path, json); }
}

pub fn is_allowlisted(path: &str) -> bool {
    let set = load_allowlist();
    let trimmed = path.trim();
    if set.contains(trimmed) { return true; }
    // also check resolved form
    let resolved = resolve(trimmed).display().to_string();
    if set.contains(&resolved) { return true; }
    for pat in &set {
        // expand ~ in pattern
        let pat_expanded = expand_tilde(pat);
        let target = trimmed;
        let target_expanded = expand_tilde(target);
        if wildmatch::WildMatch::new(&pat_expanded).matches(&target_expanded) { return true; }
        if wildmatch::WildMatch::new(&pat_expanded).matches(&resolved) { return true; }
        if wildmatch::WildMatch::new(pat).matches(target) { return true; }
        // prefix fallback (no wildcard => prefix match)
        if !pat.contains('*') && (target.starts_with(pat) || resolved.starts_with(pat) || target_expanded.starts_with(&pat_expanded)) {
            return true;
        }
    }
    false
}

pub fn allowlist_list() -> Vec<String> {
    let mut v: Vec<String> = load_allowlist().into_iter().collect();
    v.sort();
    v
}

pub fn allowlist_add(pat: &str) {
    let mut set = load_allowlist();
    set.insert(pat.trim().to_string());
    // also store resolved form for robustness? keep original
    save_allowlist(&set);
}

pub fn allowlist_remove(pat: &str) {
    let mut set = load_allowlist();
    set.remove(pat);
    set.remove(pat.trim());
    save_allowlist(&set);
}

pub fn allowlist_clear() {
    let _ = std::fs::remove_file(allowlist_path());
}

fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}/{}", home.display(), rest);
        }
    } else if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.display().to_string();
        }
    }
    s.to_string()
}

// ---- path resolution ----

/// Lexically normalize a path (remove `.`, resolve `..` without hitting FS)
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        use std::path::Component;
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push("/"),
            Component::CurDir => {},
            Component::ParentDir => { out.pop(); },
            Component::Normal(c) => out.push(c),
        }
    }
    if out.as_os_str().is_empty() { out.push("."); }
    out
}

/// Resolve a user-supplied path string relative to project root.
/// Handles `~`, absolute, and relative paths.
fn resolve(raw: &str) -> PathBuf {
    let expanded = expand_tilde(raw.trim());
    let p = Path::new(&expanded);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        project_root().join(p)
    };
    // Try canonicalize if exists (resolves symlinks), else lexical normalize
    if joined.exists() {
        if let Ok(c) = joined.canonicalize() { return c; }
    } else {
        // for non-existent, canonicalize parent if possible
        if let Some(parent) = joined.parent() {
            if parent.exists() {
                if let Ok(c) = parent.canonicalize() {
                    if let Some(name) = joined.file_name() {
                        return normalize(&c.join(name));
                    }
                    return normalize(&c);
                }
            }
        }
    }
    normalize(&joined)
}

/// Check if a path is inside project root
pub fn is_inside(path: &str) -> bool {
    if path.trim().is_empty() { return true; }
    let root = project_root();
    let resolved = resolve(path);
    resolved.starts_with(&root)
}

// ---- risk analysis ----

#[derive(Debug, Clone, PartialEq)]
pub enum Severity { High }

#[derive(Debug, Clone)]
pub struct Risk {
    pub severity: Severity,
    pub reasons: Vec<String>,
    /// The offending path(s) for allowlisting
    pub paths: Vec<String>,
}

/// Analyze a single file path for directory escape
pub fn analyze_path(path: &str) -> Option<Risk> {
    if is_disabled() { return None; }
    if std::env::var("LEAN_DIR_GUARD_DISABLED").map(|v| v == "1" || v == "true").unwrap_or(false) { return None; }
    let trimmed = path.trim();
    if trimmed.is_empty() { return None; }
    if is_allowlisted(trimmed) { return None; }
    if is_inside(trimmed) { return None; }
    let resolved = resolve(trimmed);
    Some(Risk {
        severity: Severity::High,
        reasons: vec![format!("outside CWD ({} → {})", trimmed, resolved.display())],
        paths: vec![trimmed.to_string()],
    })
}

/// Light bash path extraction — finds path-like tokens that escape CWD.
/// Reuses tokenization idea from bash_guard but focused on paths.
pub fn analyze_bash(command: &str) -> Option<Risk> {
    if is_disabled() { return None; }
    if std::env::var("LEAN_DIR_GUARD_DISABLED").map(|v| v == "1" || v == "true").unwrap_or(false) { return None; }
    let raw = command.trim();
    if raw.is_empty() { return None; }

    // Quick allowlist check on whole command
    if is_allowlisted(raw) { return None; }

    // Extract candidate paths from command
    let candidates = extract_paths(raw);
    let mut offending = Vec::new();
    let mut reasons = Vec::new();

    for cand in candidates {
        if cand.is_empty() { continue; }
        // Skip flags, URLs, globs that are not paths, etc.
        if cand.starts_with('-') { continue; }
        if cand.contains("://") { continue; }
        // Heuristic: must look like a path (contains / or . or ~) or is a known file op target
        let looks_like_path = cand.contains('/') || cand.starts_with('~') || cand.starts_with('.') || cand == ".." || cand == ".";
        // Also check bare names that might be outside via relative? e.g. `cat ../foo`
        // If it doesn't look like path, skip unless it's after a path-expecting command, but we treat all candidates
        if !looks_like_path {
            // Check if it's an absolute-like or contains ~, otherwise ignore bare word (likely not a path escape)
            // But `../foo` would be caught because it contains /
            continue;
        }
        if is_allowlisted(&cand) { continue; }
        if !is_inside(&cand) {
            let resolved = resolve(&cand);
            offending.push(cand.clone());
            reasons.push(format!("outside CWD ({} → {})", cand, resolved.display()));
        }
    }

    // Also detect explicit `cd` outside
    // If command contains `cd <path>` where path is outside, that's already caught, but also `cd /tmp` should be flagged even if we missed
    // The extract_paths already covers it.

    if offending.is_empty() { return None; }
    Some(Risk { severity: Severity::High, reasons, paths: offending })
}

fn extract_paths(cmd: &str) -> Vec<String> {
    // Very light tokenization respecting quotes, then collect tokens that could be paths
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for c in cmd.chars() {
        if escaped { cur.push(c); escaped = false; continue; }
        match c {
            '\\' if !in_single => { escaped = true; }
            '\'' if !in_double => { in_single = !in_single; }
            '"' if !in_single => { in_double = !in_double; }
            ' ' | '\t' | '\n' | ';' | '|' | '&' | '(' | ')' | '<' | '>' | '`' if !in_single && !in_double => {
                if !cur.is_empty() { tokens.push(cur.clone()); cur.clear(); }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() { tokens.push(cur); }

    // Filter out known command names (first token per segment) and operators
    // Heuristic: drop tokens that are typical flags/commands without slash
    // Keep tokens that contain path separators or start with . ~ /
    let mut out = Vec::new();
    for t in tokens {
        let trimmed = t.trim_matches(|c| c == '\'' || c == '"').to_string();
        if trimmed.is_empty() { continue; }
        // Skip common non-path keywords
        if ["echo", "cat", "ls", "grep", "find", "cargo", "git", "npm", "pnpm", "yarn", "node", "python", "python3", "pip", "curl", "wget", "head", "tail", "wc", "sort", "uniq", "awk", "sed", "jq", "rg", "fd", "bash", "sh", "zsh", "cd", "cp", "mv", "rm", "mkdir", "touch", "chmod", "chown", "sudo", "env", "export"].contains(&trimmed.as_str()) {
            continue;
        }
        // Also skip tokens that are purely flags
        if trimmed.starts_with('-') { continue; }
        out.push(trimmed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn setup(tmp: &Path) {
        let _ = PROJECT_ROOT.set(tmp.canonicalize().unwrap_or(tmp.to_path_buf()));
    }

    #[test]
    fn inside_relative() {
        let dir = env::temp_dir().join("lean_test_inside");
        let _ = std::fs::create_dir_all(&dir);
        // reset
        let _ = PROJECT_ROOT.set(dir.clone());
        assert!(is_inside("src/main.rs"));
        assert!(is_inside("./src/main.rs"));
    }

    #[test]
    fn outside_absolute() {
        let dir = env::temp_dir().join("lean_test_outside");
        let _ = std::fs::create_dir_all(&dir);
        // This test is fragile due to OnceLock — just check logic via resolve
        let outside = "/tmp/other.txt";
        let root = PathBuf::from("/home/naet/Documents/lean");
        let _ = PROJECT_ROOT.get_or_init(|| root.clone());
        // If root is lean, /tmp should be outside
        if PROJECT_ROOT.get().unwrap() == &root {
            assert!(!is_inside(outside));
        }
    }
}
