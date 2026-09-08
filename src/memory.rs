use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

const MAX_MEMORIES: usize = 500;
const RECALL_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub category: String,
    pub tags: Vec<String>,
    pub scope: String,
    pub created_at: String,
}

impl MemoryEntry {
    fn new(content: &str, category: &str, tags: Vec<String>, scope: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id: format!("mem_{}", now),
            content: content.to_string(),
            category: category.to_string(),
            tags,
            scope: scope.to_string(),
            created_at: format!("{now}"),
        }
    }
}

pub struct MemoryStore {
    entries: Vec<MemoryEntry>,
    path: PathBuf,
}

static STORE: OnceLock<std::sync::Mutex<MemoryStore>> = OnceLock::new();

fn memory_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("memory.json")
}

fn store_lock() -> &'static std::sync::Mutex<MemoryStore> {
    STORE.get_or_init(|| {
        let path = memory_path();
        let entries = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        std::sync::Mutex::new(MemoryStore { entries, path })
    })
}

impl MemoryStore {
    fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&self.entries).unwrap_or_default();
        let _ = std::fs::write(&self.path, json);
    }

    fn remember(&mut self, content: &str, category: &str, tags: Vec<String>, scope: &str) -> String {
        let entry = MemoryEntry::new(content, category, tags, scope);
        let id = entry.id.clone();
        self.entries.push(entry);

        // Evict oldest if over limit
        if self.entries.len() > MAX_MEMORIES {
            let excess = self.entries.len() - MAX_MEMORIES;
            self.entries.drain(..excess);
        }

        self.persist();
        format!("Stored memory: {}", id)
    }

    fn search(&self, query: &str) -> Vec<&MemoryEntry> {
        let keywords = extract_keywords(query);
        if keywords.is_empty() {
            return self.recall();
        }

        let mut scored: Vec<(&MemoryEntry, f32)> = self
            .entries
            .iter()
            .filter(|e| {
                // Global or project-scoped only
                e.scope == "global" || e.scope == "project"
            })
            .map(|e| {
                let score = score_entry(e, &keywords);
                (e, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(RECALL_LIMIT).map(|(e, _)| e).collect()
    }

    fn recall(&self) -> Vec<&MemoryEntry> {
        // Return most recent memories
        self.entries
            .iter()
            .rev()
            .take(RECALL_LIMIT)
            .collect()
    }

    fn list_by_tag(&self, tag: &str) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.tags.iter().any(|t| t == tag))
            .collect()
    }

    fn forget(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() < before;
        if removed {
            self.persist();
        }
        removed
    }
}

fn extract_keywords(text: &str) -> Vec<String> {
    let stop = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "shall", "can", "need", "dare", "ought",
        "used", "to", "of", "in", "for", "on", "with", "at", "by", "from",
        "as", "into", "through", "during", "before", "after", "above", "below",
        "between", "out", "off", "over", "under", "again", "further", "then",
        "once", "here", "there", "when", "where", "why", "how", "all", "each",
        "every", "both", "few", "more", "most", "other", "some", "such", "no",
        "nor", "not", "only", "own", "same", "so", "than", "too", "very",
        "just", "because", "but", "and", "or", "if", "while", "about",
        "it", "its", "this", "that", "these", "those", "i", "you", "he",
        "she", "we", "they", "me", "him", "her", "us", "them", "my",
        "your", "his", "our", "what", "which", "who", "whom",
    ];

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !stop.contains(&w.as_ref()))
        .map(|w| w.to_string())
        .collect()
}

fn score_entry(entry: &MemoryEntry, keywords: &[String]) -> f32 {
    let content_lower = entry.content.to_lowercase();
    let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();

    let mut score = 0.0f32;
    for kw in keywords {
        // Content match (full word)
        let pattern = format!(" {} ", kw);
        if content_lower.contains(&pattern) {
            score += 2.0;
        } else if content_lower.contains(kw) {
            score += 1.0;
        }
        // Tag match
        if tags_lower.iter().any(|t| t.contains(kw)) {
            score += 3.0;
        }
        // Category match
        if entry.category.to_lowercase().contains(kw) {
            score += 1.5;
        }
    }
    score
}

// ---- Public API (called from tools) ----

pub fn api_remember(content: &str, category: &str, tags: Vec<String>, scope: &str) -> String {
    let mut lock = store_lock().lock().unwrap();
    lock.remember(content, category, tags, scope)
}

pub fn api_search(query: &str) -> String {
    let lock = store_lock().lock().unwrap();
    let results = lock.search(query);
    format_memories(&results)
}

pub fn api_recall() -> String {
    let lock = store_lock().lock().unwrap();
    let results = lock.recall();
    format_memories(&results)
}

pub fn api_list(tag: &str) -> String {
    let lock = store_lock().lock().unwrap();
    let results = lock.list_by_tag(tag);
    format_memories(&results)
}

pub fn api_forget(id: &str) -> String {
    let mut lock = store_lock().lock().unwrap();
    if lock.forget(id) {
        format!("Forgot memory: {}", id)
    } else {
        format!("Memory not found: {}", id)
    }
}

/// Autorecall: search memories based on recent conversation context.
/// Returns a prompt fragment to inject, or None if nothing relevant.
pub fn autorecall(context: &str) -> Option<String> {
    let lock = store_lock().lock().unwrap();
    let results = lock.search(context);
    if results.is_empty() {
        return None;
    }

    let mut out = String::from("Recalled memories for context:\n");
    for (i, m) in results.iter().enumerate() {
        out.push_str(&format!("  {}. [{}] {}\n", i + 1, m.category, m.content));
    }
    Some(out)
}

fn format_memories(entries: &[&MemoryEntry]) -> String {
    if entries.is_empty() {
        return "No memories found.".to_string();
    }
    let mut out = String::new();
    for (i, m) in entries.iter().enumerate() {
        out.push_str(&format!(
            "[{}] {} (id: {}, tags: [{}], scope: {})\n",
            m.category,
            m.content,
            m.id,
            m.tags.join(", "),
            m.scope,
        ));
        let _ = i;
    }
    out
}
