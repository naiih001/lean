pub mod recall;
pub mod store;

pub use recall::{extract_keywords, format_memories, score_entry};
pub use store::{MemoryEntry, MemoryStore};

use recall::autorecall_context;
use std::time::{SystemTime, UNIX_EPOCH};
use store::store_lock;

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

pub fn autorecall(context: &str) -> Option<String> {
    let lock = store_lock().lock().unwrap();
    let results = lock.search(context);
    autorecall_context(context, &results)
}

pub fn format_memories_wrapper(entries: &[&MemoryEntry]) -> String {
    format_memories(entries)
}
