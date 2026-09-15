// Shared allowlist persistence — extracted from bash_guard/dir_guard duplication.
use std::collections::HashSet;
use std::path::PathBuf;

fn base_path(name: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join(name)
}

pub fn load(name: &str) -> HashSet<String> {
    let path = base_path(name);
    if let Ok(txt) = std::fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(&txt) {
            return v.into_iter().collect();
        }
    }
    HashSet::new()
}

pub fn save(name: &str, set: &HashSet<String>) {
    let path = base_path(name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let vec: Vec<String> = set.iter().cloned().collect();
    if let Ok(json) = serde_json::to_string_pretty(&vec) {
        let _ = std::fs::write(path, json);
    }
}

pub fn list(name: &str) -> Vec<String> {
    let mut v: Vec<String> = load(name).into_iter().collect();
    v.sort();
    v
}

pub fn add(name: &str, pat: &str) {
    let mut set = load(name);
    set.insert(pat.trim().to_string());
    save(name, &set);
}

pub fn remove(name: &str, pat: &str) {
    let mut set = load(name);
    set.remove(pat);
    set.remove(pat.trim());
    save(name, &set);
}

pub fn clear(name: &str) {
    save(name, &HashSet::new());
}

pub fn is_match(set: &HashSet<String>, trimmed: &str) -> bool {
    if set.contains(trimmed) {
        return true;
    }
    for pat in set {
        if wildmatch::WildMatch::new(pat).matches(trimmed) {
            return true;
        }
        if !pat.contains('*') && trimmed.starts_with(pat.as_str()) {
            return true;
        }
    }
    false
}
