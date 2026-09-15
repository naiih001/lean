use crate::services::memory::recall::{extract_keywords, score_entry};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const MAX_MEMORIES: usize = 500;
pub(crate) const RECALL_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub category: String,
    pub tags: Vec<String>,
    pub scope: String,
    pub created_at: String,
    #[serde(default)]
    pub ts: Option<u64>,
}

impl MemoryEntry {
    pub(crate) fn new(content: &str, category: &str, tags: Vec<String>, scope: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let nanos = now.subsec_nanos();
        let suffix: u32 = (nanos
            ^ std::process::id().wrapping_mul(0x9e3779b1)
            ^ (now.as_millis() as u32).wrapping_mul(0x85ebca6b))
            % 0xFFFFFF;
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

    pub(crate) fn ts_secs(&self) -> u64 {
        if let Some(t) = self.ts {
            return t;
        }
        self.created_at.parse::<u64>().unwrap_or(0)
    }
}

pub struct MemoryStore {
    pub(crate) entries: Vec<MemoryEntry>,
    pub(crate) path: PathBuf,
}

static STORE: OnceLock<std::sync::Mutex<MemoryStore>> = OnceLock::new();

pub(crate) fn memory_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".lean")
        .join("memory.json")
}

pub(crate) fn store_lock() -> &'static std::sync::Mutex<MemoryStore> {
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
    pub(crate) fn persist(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&self.entries).unwrap_or_default();
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        } else {
            let _ = std::fs::write(&self.path, json);
        }
    }

    pub(crate) fn remember(
        &mut self,
        content: &str,
        category: &str,
        tags: Vec<String>,
        scope: &str,
    ) -> String {
        let trimmed = content.trim();
        if trimmed.len() < 3 {
            return "Memory too short, ignored".to_string();
        }
        let lower = trimmed.to_lowercase();
        if self
            .entries
            .iter()
            .rev()
            .take(20)
            .any(|e| e.content.to_lowercase() == lower)
        {
            return "Memory already stored (duplicate)".to_string();
        }
        let mut norm_tags: Vec<String> = tags
            .into_iter()
            .map(|t| t.to_lowercase().trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        norm_tags.sort();
        norm_tags.dedup();
        let entry = MemoryEntry::new(trimmed, category, norm_tags, scope);
        let id = entry.id.clone();
        self.entries.push(entry);
        if self.entries.len() > MAX_MEMORIES {
            let excess = self.entries.len() - MAX_MEMORIES;
            self.entries.drain(..excess);
        }
        self.persist();
        format!("Stored memory: {}", id)
    }

    pub(crate) fn search(&self, query: &str) -> Vec<&MemoryEntry> {
        let keywords = extract_keywords(query);
        if keywords.is_empty() {
            return self.recall();
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut scored: Vec<(&MemoryEntry, f32)> = self
            .entries
            .iter()
            .filter(|e| e.scope == "global" || e.scope == "project")
            .map(|e| {
                let base = score_entry(e, &keywords);
                let age_secs = now.saturating_sub(e.ts_secs());
                let recency = if age_secs < 7 * 86400 {
                    0.8
                } else if age_secs < 30 * 86400 {
                    0.4
                } else {
                    0.0
                };
                let len_penalty = if e.content.len() < 20 { -0.3 } else { 0.0 };
                (e, base + recency + len_penalty)
            })
            .filter(|(_, score)| *score > 0.5)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(RECALL_LIMIT)
            .map(|(e, _)| e)
            .collect()
    }

    pub(crate) fn recall(&self) -> Vec<&MemoryEntry> {
        self.entries.iter().rev().take(RECALL_LIMIT).collect()
    }

    pub(crate) fn list_by_tag(&self, tag: &str) -> Vec<&MemoryEntry> {
        let lower = tag.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.tags.iter().any(|t| t.to_lowercase() == lower))
            .collect()
    }

    pub(crate) fn forget(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() < before;
        if removed {
            self.persist();
        }
        removed
    }

    pub(crate) fn consolidate(&mut self) -> String {
        let before = self.entries.len();
        if before < 2 {
            return "No consolidation needed".to_string();
        }
        let mut to_remove = HashSet::new();
        let mut merges = 0;
        for i in 0..self.entries.len() {
            if to_remove.contains(&i) {
                continue;
            }
            let kw_i = extract_keywords(&self.entries[i].content)
                .into_iter()
                .collect::<HashSet<_>>();
            if kw_i.is_empty() {
                continue;
            }
            for j in (i + 1)..self.entries.len() {
                if to_remove.contains(&j) {
                    continue;
                }
                let kw_j = extract_keywords(&self.entries[j].content)
                    .into_iter()
                    .collect::<HashSet<_>>();
                if kw_j.is_empty() {
                    continue;
                }
                let inter = kw_i.intersection(&kw_j).count() as f32;
                let union = kw_i.union(&kw_j).count() as f32;
                let jaccard = inter / union;
                if jaccard > 0.75 {
                    let mut tags: HashSet<String> = self.entries[i].tags.iter().cloned().collect();
                    tags.extend(self.entries[j].tags.iter().cloned());
                    let mut v: Vec<String> = tags.into_iter().collect();
                    v.sort();
                    self.entries[i].tags = v;
                    if self.entries[j].content.len() > self.entries[i].content.len()
                        && self.entries[j].ts_secs() >= self.entries[i].ts_secs()
                    {
                        self.entries[i].content = self.entries[j].content.clone();
                    }
                    to_remove.insert(j);
                    merges += 1;
                }
            }
        }
        if !to_remove.is_empty() {
            let mut idxs: Vec<usize> = to_remove.into_iter().collect();
            idxs.sort_by(|a, b| b.cmp(a));
            for idx in idxs {
                self.entries.remove(idx);
            }
            self.persist();
        }
        let after = self.entries.len();
        format!(
            "Consolidated: {} merges, {} -> {} (removed {} duplicates)",
            merges,
            before,
            after,
            before - after
        )
    }

    pub(crate) fn stats(&self) -> String {
        let total = self.entries.len();
        let mut by_cat: HashMap<String, usize> = HashMap::new();
        let mut by_scope: HashMap<String, usize> = HashMap::new();
        for e in &self.entries {
            *by_cat.entry(e.category.clone()).or_insert(0) += 1;
            *by_scope.entry(e.scope.clone()).or_insert(0) += 1;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let recent = self
            .entries
            .iter()
            .filter(|e| now.saturating_sub(e.ts_secs()) < 7 * 86400)
            .count();
        let mut out = format!("Memory stats: total {} (recent 7d: {})\n", total, recent);
        out.push_str(" by category:\n");
        let mut cats: Vec<_> = by_cat.into_iter().collect();
        cats.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, v) in cats {
            out.push_str(&format!("  {}: {}\n", k, v));
        }
        out.push_str(" by scope:\n");
        let mut scopes: Vec<_> = by_scope.into_iter().collect();
        scopes.sort_by(|a, b| b.1.cmp(&a.1));
        for (k, v) in scopes {
            out.push_str(&format!("  {}: {}\n", k, v));
        }
        out
    }
}
