use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::services::memory::store::MemoryEntry;

pub fn extract_keywords(text: &str) -> Vec<String> {
    let stop: HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "need", "dare", "ought", "used", "to", "of", "in", "for", "on", "with", "at", "by", "from",
        "as", "into", "through", "during", "before", "after", "above", "below", "between", "out",
        "off", "over", "under", "again", "further", "then", "once", "here", "there", "when",
        "where", "why", "how", "all", "each", "every", "both", "few", "more", "most", "other",
        "some", "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too", "very",
        "just", "because", "but", "and", "or", "if", "while", "about", "it", "its", "this", "that",
        "these", "those", "i", "you", "he", "she", "we", "they", "me", "him", "her", "us", "them",
        "my", "your", "his", "our", "what", "which", "who", "whom",
    ]
    .into_iter()
    .collect();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !stop.contains(*w))
        .map(|w| w.to_string())
        .collect()
}

pub fn score_entry(entry: &MemoryEntry, keywords: &[String]) -> f32 {
    let content_lower = entry.content.to_lowercase();
    let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();
    let content_words: HashSet<String> = content_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|s| s.to_string())
        .collect();
    let mut score = 0.0f32;
    for kw in keywords {
        if content_words.contains(kw) {
            score += 2.5;
        } else if content_lower.contains(kw) {
            score += 1.0;
        }
        if tags_lower.iter().any(|t| t == kw) {
            score += 4.0;
        } else if tags_lower.iter().any(|t| t.contains(kw)) {
            score += 2.0;
        }
        if entry.category.to_lowercase() == *kw {
            score += 3.0;
        } else if entry.category.to_lowercase().contains(kw) {
            score += 1.5;
        }
        if content_lower.contains(kw) && tags_lower.iter().any(|t| t.contains(kw)) {
            score += 0.5;
        }
    }
    if keywords
        .iter()
        .all(|kw| content_lower.contains(kw) || tags_lower.iter().any(|t| t.contains(kw)))
    {
        score += 1.0;
    }
    score
}

pub fn format_memories(entries: &[&MemoryEntry]) -> String {
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
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(m.ts_secs()),
        ));
    }
    out
}

pub fn autorecall_context(context: &str, entries: &[&MemoryEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut out = String::from("Recalled memories for context:\n");
    for (i, m) in entries.iter().enumerate() {
        let age = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(m.ts_secs());
        let age_hint = if age < 86400 {
            "today"
        } else if age < 7 * 86400 {
            "this week"
        } else if age < 30 * 86400 {
            "this month"
        } else {
            "older"
        };
        out.push_str(&format!(
            "  {}. [{}|{}|{}] {}\n",
            i + 1,
            m.category,
            m.scope,
            age_hint,
            m.content
        ));
    }
    Some(out)
}
