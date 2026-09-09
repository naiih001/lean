use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

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
    // internal numeric for recency sorting (not serialized as string for compat, keep string)
    #[serde(default)]
    pub ts: Option<u64>,
}

impl MemoryEntry {
    fn new(content: &str, category: &str, tags: Vec<String>, scope: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let nanos = now.subsec_nanos();
        // unique suffix from thread + nanos low bits + pid mixing
        let suffix: u32 = (nanos ^ std::process::id().wrapping_mul(0x9e3779b1) ^ (now.as_millis() as u32).wrapping_mul(0x85ebca6b)) % 0xFFFFFF;
        let id = format!("mem_{}_{:06x}", secs, suffix);
        Self {
            id,
            content: content.to_string(),
            category: category.to_string(),
            tags,
            scope: scope.to_string(),
            created_at: format!("{secs}"),
            ts: Some(secs),
        }
    }

    fn ts_secs(&self) -> u64 {
        if let Some(t) = self.ts { return t; }
        self.created_at.parse::<u64>().unwrap_or(0)
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
        // atomic write: tmp then rename
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        } else {
            let _ = std::fs::write(&self.path, json);
        }
    }

    fn remember(&mut self, content: &str, category: &str, tags: Vec<String>, scope: &str) -> String {
        let trimmed = content.trim();
        if trimmed.len() < 3 {
            return "Memory too short, ignored".to_string();
        }
        // Deduplicate exact content (case-insensitive) within last 20
        let lower = trimmed.to_lowercase();
        if self.entries.iter().rev().take(20).any(|e| e.content.to_lowercase() == lower) {
            return "Memory already stored (duplicate)".to_string();
        }
        // Normalize tags: lowercase, dedup
        let mut norm_tags: Vec<String> = tags.into_iter().map(|t| t.to_lowercase().trim().to_string()).filter(|t| !t.is_empty()).collect();
        norm_tags.sort();
        norm_tags.dedup();
        let entry = MemoryEntry::new(trimmed, category, norm_tags, scope);
        let id = entry.id.clone();
        self.entries.push(entry);

        // Evict oldest if over limit (keep most recent)
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

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let mut scored: Vec<(&MemoryEntry, f32)> = self
            .entries
            .iter()
            .filter(|e| e.scope == "global" || e.scope == "project")
            .map(|e| {
                let base = score_entry(e, &keywords);
                // Recency boost: within 7 days +0.8, 30 days +0.4
                let age_secs = now.saturating_sub(e.ts_secs());
                let recency = if age_secs < 7 * 86400 { 0.8 } else if age_secs < 30 * 86400 { 0.4 } else { 0.0 };
                // Length penalty for very short memories
                let len_penalty = if e.content.len() < 20 { -0.3 } else { 0.0 };
                (e, base + recency + len_penalty)
            })
            .filter(|(_, score)| *score > 0.5)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(RECALL_LIMIT).map(|(e, _)| e).collect()
    }

    fn recall(&self) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .rev()
            .take(RECALL_LIMIT)
            .collect()
    }

    fn list_by_tag(&self, tag: &str) -> Vec<&MemoryEntry> {
        let lower = tag.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.tags.iter().any(|t| t.to_lowercase() == lower))
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

    fn consolidate(&mut self) -> String {
        let before = self.entries.len();
        if before < 2 {
            return "No consolidation needed".to_string();
        }
        // Jaccard similarity on keyword sets to merge near-duplicates
        let mut to_remove = HashSet::new();
        let mut merges = 0;
        for i in 0..self.entries.len() {
            if to_remove.contains(&i) { continue; }
            let kw_i = extract_keywords(&self.entries[i].content).into_iter().collect::<HashSet<_>>();
            if kw_i.is_empty() { continue; }
            for j in (i+1)..self.entries.len() {
                if to_remove.contains(&j) { continue; }
                let kw_j = extract_keywords(&self.entries[j].content).into_iter().collect::<HashSet<_>>();
                if kw_j.is_empty() { continue; }
                let inter = kw_i.intersection(&kw_j).count() as f32;
                let union = kw_i.union(&kw_j).count() as f32;
                let jaccard = inter / union;
                if jaccard > 0.75 {
                    // Merge j into i: union tags, keep newer content if longer
                    let mut tags: HashSet<String> = self.entries[i].tags.iter().cloned().collect();
                    tags.extend(self.entries[j].tags.iter().cloned());
                    let mut v: Vec<String> = tags.into_iter().collect();
                    v.sort();
                    self.entries[i].tags = v;
                    // If j is longer and newer, replace content
                    if self.entries[j].content.len() > self.entries[i].content.len() && self.entries[j].ts_secs() >= self.entries[i].ts_secs() {
                        self.entries[i].content = self.entries[j].content.clone();
                    }
                    to_remove.insert(j);
                    merges += 1;
                }
            }
        }
        if !to_remove.is_empty() {
            let mut idxs: Vec<usize> = to_remove.into_iter().collect();
            idxs.sort_by(|a,b| b.cmp(a));
            for idx in idxs { self.entries.remove(idx); }
            self.persist();
        }
        let after = self.entries.len();
        format!("Consolidated: {} merges, {} -> {} (removed {} duplicates)", merges, before, after, before - after)
    }

    fn stats(&self) -> String {
        let total = self.entries.len();
        let mut by_cat: HashMap<String, usize> = HashMap::new();
        let mut by_scope: HashMap<String, usize> = HashMap::new();
        for e in &self.entries { *by_cat.entry(e.category.clone()).or_insert(0) += 1; *by_scope.entry(e.scope.clone()).or_insert(0) += 1; }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let recent = self.entries.iter().filter(|e| now.saturating_sub(e.ts_secs()) < 7*86400).count();
        let mut out = format!("Memory stats: total {} (recent 7d: {})\n", total, recent);
        out.push_str(" by category:\n");
        let mut cats: Vec<_> = by_cat.into_iter().collect(); cats.sort_by(|a,b| b.1.cmp(&a.1));
        for (k,v) in cats { out.push_str(&format!("  {}: {}\n", k, v)); }
        out.push_str(" by scope:\n");
        let mut scopes: Vec<_> = by_scope.into_iter().collect(); scopes.sort_by(|a,b| b.1.cmp(&a.1));
        for (k,v) in scopes { out.push_str(&format!("  {}: {}\n", k, v)); }
        out
    }
}

fn extract_keywords(text: &str) -> Vec<String> {
    let stop: HashSet<&str> = [
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
    ].into_iter().collect();

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !stop.contains(*w))
        .map(|w| w.to_string())
        .collect()
}

fn score_entry(entry: &MemoryEntry, keywords: &[String]) -> f32 {
    let content_lower = entry.content.to_lowercase();
    let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();
    let content_words: HashSet<String> = content_lower.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len()>2).map(|s| s.to_string()).collect();

    let mut score = 0.0f32;
    for kw in keywords {
        // Exact word in content
        if content_words.contains(kw) {
            score += 2.5;
        } else if content_lower.contains(kw) {
            score += 1.0;
        }
        // Tag exact
        if tags_lower.iter().any(|t| t == kw) {
            score += 4.0;
        } else if tags_lower.iter().any(|t| t.contains(kw)) {
            score += 2.0;
        }
        // Category
        if entry.category.to_lowercase() == *kw {
            score += 3.0;
        } else if entry.category.to_lowercase().contains(kw) {
            score += 1.5;
        }
        // Bonus for multi-keyword coverage
        if content_lower.contains(kw) && tags_lower.iter().any(|t| t.contains(kw)) {
            score += 0.5;
        }
    }
    // Coverage boost: if all keywords appear, +1
    if keywords.iter().all(|kw| content_lower.contains(kw) || tags_lower.iter().any(|t| t.contains(kw))) {
        score += 1.0;
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

pub fn api_consolidate() -> String {
    let mut lock = store_lock().lock().unwrap();
    lock.consolidate()
}

pub fn api_stats() -> String {
    let lock = store_lock().lock().unwrap();
    lock.stats()
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
        // Include scope and recency hint for LLM
        let age = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(m.ts_secs());
        let age_hint = if age < 86400 { "today" } else if age < 7*86400 { "this week" } else if age < 30*86400 { "this month" } else { "older" };
        out.push_str(&format!("  {}. [{}|{}|{}] {}\n", i + 1, m.category, m.scope, age_hint, m.content));
    }
    Some(out)
}

fn format_memories(entries: &[&MemoryEntry]) -> String {
    if entries.is_empty() {
        return "No memories found.".to_string();
    }
    let mut out = String::new();
    for m in entries.iter() {
        out.push_str(&format!(
            "[{}] {} (id: {}, tags: [{}], scope: {}, age: {}s)\n",
            m.category,
            m.content,
            m.id,
            m.tags.join(", "),
            m.scope,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().saturating_sub(m.ts_secs()),
        ));
    }
    out
}
