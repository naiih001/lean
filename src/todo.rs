use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoStatus::Pending => write!(f, "pending"),
            TodoStatus::InProgress => write!(f, "in_progress"),
            TodoStatus::Completed => write!(f, "completed"),
            TodoStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
    pub priority: String,
    pub group: String,
    pub created_at: String,
}

pub struct TodoStore {
    items: Vec<TodoItem>,
    next_id: usize,
}

static STORE: OnceLock<std::sync::Mutex<TodoStore>> = OnceLock::new();

fn store_lock() -> &'static std::sync::Mutex<TodoStore> {
    STORE.get_or_init(|| {
        std::sync::Mutex::new(TodoStore {
            items: Vec::new(),
            next_id: 1,
        })
    })
}

impl TodoStore {
    fn add(
        &mut self,
        content: &str,
        priority: &str,
        group: &str,
    ) -> String {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let item = TodoItem {
            id: format!("todo_{}", self.next_id),
            content: content.to_string(),
            status: TodoStatus::Pending,
            priority: priority.to_string(),
            group: group.to_string(),
            created_at: format!("{now}"),
        };
        self.next_id += 1;
        let id = item.id.clone();
        self.items.push(item);
        id
    }

    fn update(&mut self, id: &str, status: &str) -> Result<String, String> {
        let item = self
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("Todo not found: {}", id))?;
        item.status = match status {
            "pending" => TodoStatus::Pending,
            "in_progress" => TodoStatus::InProgress,
            "completed" => TodoStatus::Completed,
            "cancelled" => TodoStatus::Cancelled,
            _ => return Err(format!("Unknown status: {}", status)),
        };
        Ok(format!("Updated {} to {}", id, item.status))
    }

    fn remove(&mut self, id: &str) -> Result<String, String> {
        let before = self.items.len();
        self.items.retain(|i| i.id != id);
        if self.items.len() < before {
            Ok(format!("Removed: {}", id))
        } else {
            Err(format!("Todo not found: {}", id))
        }
    }

    fn list(&self) -> &[TodoItem] {
        &self.items
    }

    fn format_list(&self) -> String {
        if self.items.is_empty() {
            return "No todos.".to_string();
        }

        // Group items
        let mut groups: Vec<(&str, Vec<&TodoItem>)> = Vec::new();
        let mut ungrouped: Vec<&TodoItem> = Vec::new();

        for item in &self.items {
            if item.group.is_empty() {
                ungrouped.push(item);
            } else {
                if let Some((_, g)) = groups.iter_mut().find(|(name, _)| *name == item.group.as_str())
                {
                    g.push(item);
                } else {
                    groups.push((item.group.as_str(), vec![item]));
                }
            }
        }

        let mut out = String::new();

        for (group_name, items) in &groups {
            out.push_str(&format!("## {}\n", group_name));
            for item in items {
                out.push_str(&format!("{}\n", format_item(item)));
            }
            out.push('\n');
        }

        if !ungrouped.is_empty() {
            out.push_str("## General\n");
            for item in &ungrouped {
                out.push_str(&format!("{}\n", format_item(item)));
            }
        }

        out
    }
}

fn format_item(item: &TodoItem) -> String {
    let check = match item.status {
        TodoStatus::Completed => "x",
        TodoStatus::Cancelled => "-",
        TodoStatus::InProgress => "~",
        TodoStatus::Pending => " ",
    };
    let priority = match item.priority.as_str() {
        "high" => "!!",
        "medium" => "!",
        _ => " ",
    };
    format!("[{}] {}{} {}", check, item.content, if priority == " " { "".to_string() } else { format!(" ({})", priority) }, item.id)
}

// ---- Public API ----

pub fn api_add(content: &str, priority: &str, group: &str) -> String {
    let mut lock = store_lock().lock().unwrap();
    let id = lock.add(content, priority, group);
    format!("Created todo: {}", id)
}

pub fn api_update(id: &str, status: &str) -> String {
    let mut lock = store_lock().lock().unwrap();
    match lock.update(id, status) {
        Ok(msg) => msg,
        Err(e) => e,
    }
}

pub fn api_remove(id: &str) -> String {
    let mut lock = store_lock().lock().unwrap();
    match lock.remove(id) {
        Ok(msg) => msg,
        Err(e) => e,
    }
}

pub fn api_list() -> String {
    let lock = store_lock().lock().unwrap();
    lock.format_list()
}

/// Get todos for TUI rendering (used by /todo popup)
pub fn get_todos() -> Vec<TodoItem> {
    let lock = store_lock().lock().unwrap();
    lock.list().to_vec()
}
