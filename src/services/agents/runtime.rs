use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct SubagentMsg {
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<String>,
    pub tool_id: Option<String>,
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SubagentStatus {
    pub id: String,
    pub agent: String,
    pub label: String,
    pub task: String,
    pub status: String,
    pub started_at: SystemTime,
    pub transcript: Vec<SubagentMsg>,
}

static SUBAGENTS: OnceLock<Mutex<Vec<SubagentStatus>>> = OnceLock::new();
fn subagents_lock() -> &'static Mutex<Vec<SubagentStatus>> {
    SUBAGENTS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn list_subagents() -> Vec<SubagentStatus> {
    subagents_lock().lock().unwrap().clone()
}

pub fn register_subagent(id: String, agent: String, label: String, task: String) {
    let mut lock = subagents_lock().lock().unwrap();
    lock.push(SubagentStatus {
        id,
        agent,
        label,
        task,
        status: "running".to_string(),
        started_at: SystemTime::now(),
        transcript: Vec::new(),
    });
}

pub fn append_subagent_transcript(id: &str, line: String) {
    append_subagent_msg(
        id,
        SubagentMsg {
            role: "system".to_string(),
            content: line,
            tool_name: None,
            tool_args: None,
            tool_id: None,
            elapsed_ms: None,
        },
    );
}

pub fn append_subagent_msg(id: &str, msg: SubagentMsg) {
    let mut lock = subagents_lock().lock().unwrap();
    if let Some(s) = lock.iter_mut().find(|s| s.id == id) {
        s.transcript.push(msg);
        if s.transcript.len() > 200 {
            let excess = s.transcript.len() - 200;
            s.transcript.drain(0..excess);
        }
    }
}

pub fn kill_subagent(id: &str) -> bool {
    let mut lock = subagents_lock().lock().unwrap();
    if let Some(s) = lock.iter_mut().find(|s| s.id == id) {
        if s.status == "running" {
            s.status = "killed".to_string();
            s.transcript.push(SubagentMsg {
                role: "system".to_string(),
                content: "[killed by user]".to_string(),
                tool_name: None,
                tool_args: None,
                tool_id: None,
                elapsed_ms: None,
            });
            return true;
        }
    }
    false
}

#[derive(Debug, Clone)]
pub struct WakeMessage {
    pub id: String,
    pub agent: String,
    pub label: String,
    pub task: String,
    pub result: String,
    pub elapsed_ms: u64,
}

static WAKE_QUEUE: OnceLock<Mutex<Vec<WakeMessage>>> = OnceLock::new();
fn wake_lock() -> &'static Mutex<Vec<WakeMessage>> {
    WAKE_QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}
pub fn push_wake_tool(wake: WakeMessage) {
    wake_lock().lock().unwrap().push(wake);
}
pub fn take_wake_messages() -> Vec<WakeMessage> {
    let mut lock = wake_lock().lock().unwrap();
    let out = lock.clone();
    lock.clear();
    out
}
pub fn remove_subagent(id: &str) {
    let mut lock = subagents_lock().lock().unwrap();
    lock.retain(|s| s.id != id);
}

pub fn update_subagent(id: &str, status: &str) {
    let mut lock = subagents_lock().lock().unwrap();
    if let Some(s) = lock.iter_mut().find(|s| s.id == id) {
        s.status = status.to_string();
    }
}
