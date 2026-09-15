pub mod catalog;
pub mod runtime;

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Agent {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub content: String,
    pub body: String,
    pub tools: Option<Vec<String>>,
    pub model: Option<String>,
}

pub use catalog::{discover_agents, get_agent_catalog, load_agent};
pub use runtime::{
    append_subagent_msg, append_subagent_transcript, kill_subagent, list_subagents, push_wake_tool,
    register_subagent, remove_subagent, take_wake_messages, update_subagent, SubagentMsg,
    SubagentStatus, WakeMessage,
};
