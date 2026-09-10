use serde::Deserialize;
use serde_json::{json, Value};

pub const DEFAULT_MODEL: &str = "mimo-v2.5-free";

#[derive(Debug, Clone)]
pub struct Client {
    pub api_key: String,
    pub base_url: String,
    pub http: reqwest::Client,
}

impl Client {
    pub fn from_env() -> Self {
        let api_key = std::env::var("OPENCODE_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .unwrap_or_else(|_| "sk-test".to_string());
        let base_url = std::env::var("OPENCODE_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080/zen/v1".to_string());
        Self {
            api_key,
            base_url,
            http: reqwest::Client::builder()
                .user_agent("opencode/1.0")
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    pub fn from_resolved(resolved: &crate::models::ResolvedModel) -> Self {
        Self {
            api_key: resolved.api_key.clone(),
            base_url: resolved.base_url.clone(),
            http: reqwest::Client::builder()
                .user_agent("opencode/1.0")
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    pub fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    pub fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

pub fn native_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file from disk",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to file"}
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write content to a file (creates parent dirs)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Edit a file by replacing unique oldText with newText",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "oldText": {"type": "string"},
                        "newText": {"type": "string"}
                    },
                    "required": ["path", "oldText", "newText"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Execute a bash command",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "read_skill",
                "description": "Load a skill by name",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    },
                    "required": ["name"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "remember",
                "description": "Store a memory for future recall",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "What to remember"},
                        "category": {"type": "string", "description": "fact, preference, correction, or procedure"},
                        "tags": {"type": "array", "items": {"type": "string"}, "description": "Tags for search"},
                        "scope": {"type": "string", "description": "global or project"}
                    },
                    "required": ["content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "search_memory",
                "description": "Search stored memories by keyword",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Keywords to search for"}
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "recall_memory",
                "description": "Recall the most recent stored memories",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_memories",
                "description": "List memories by tag",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "tag": {"type": "string", "description": "Tag to filter by"}
                    },
                    "required": ["tag"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "forget_memory",
                "description": "Delete a memory by id",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Memory id to forget"}
                    },
                    "required": ["id"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "consolidate_memory",
                "description": "Consolidate duplicate memories (deduplicate via Jaccard >0.75)",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "memory_stats",
                "description": "Show memory stats by category/scope and recent count",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
    ]
}

pub async fn tool_definitions() -> Vec<Value> {
    let mut v = native_tool_definitions();
    // Merge MCP tools if initialized; fast path if not
    let mcp_tools = crate::mcp::mcp_tool_definitions().await;
    v.extend(mcp_tools);
    v
}

/// Convert chat-shaped tool definitions to Responses flattened shape.
pub fn responses_tool_definitions_from(chat_defs: &[Value]) -> Vec<Value> {
    chat_defs
        .iter()
        .map(|v| {
            let f = &v["function"];
            // If already flattened (no "function" key), pass through
            if f.is_null() || f == &Value::Null {
                return v.clone();
            }
            let name = f.get("name").cloned().unwrap_or(Value::String(String::new()));
            let desc = f
                .get("description")
                .cloned()
                .unwrap_or(Value::String(String::new()));
            let params = f.get("parameters").cloned().unwrap_or(json!({"type":"object","properties":{}}));
            json!({
                "type": "function",
                "name": name,
                "description": desc,
                "parameters": params
            })
        })
        .collect()
}

pub async fn responses_tool_definitions() -> Vec<Value> {
    let chat = native_tool_definitions();
    let mut v = responses_tool_definitions_from(&chat);
    let mcp_tools = crate::mcp::mcp_tool_definitions().await;
    v.extend(responses_tool_definitions_from(&mcp_tools));
    v
}

pub async fn tool_definitions_for(mode: &crate::models::ApiMode) -> Vec<Value> {
    match mode {
        crate::models::ApiMode::Responses => responses_tool_definitions().await,
        _ => tool_definitions().await,
    }
}

/// Translate a chat `messages` Vec into (instructions, input) for Responses API.
/// - `instructions` = concatenated system messages
/// - `input` = array of Responses input items (user messages, function calls, outputs)
pub fn chat_messages_to_responses_input(messages: &[Value], system_prompt: &str) -> (String, Vec<Value>) {
    let mut instructions = system_prompt.to_string();
    let mut system_parts: Vec<String> = Vec::new();
    for m in messages {
        if m.get("role").and_then(|r| r.as_str()) == Some("system") {
            if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
                if !s.trim().is_empty() {
                    system_parts.push(s.to_string());
                }
            }
        }
    }
    if !system_parts.is_empty() {
        instructions.push_str("\n\n");
        instructions.push_str(&system_parts.join("\n\n"));
    }
    let mut input: Vec<Value> = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "user" => {
                if let Some(c) = m.get("content") {
                    if let Some(s) = c.as_str() {
                        if !s.is_empty() {
                            input.push(json!({
                                "type": "message",
                                "role": "user",
                                "content": [{"type": "input_text", "text": s}]
                            }));
                        }
                    } else if let Some(arr) = c.as_array() {
                        // multimodal chat content -> map to input content parts
                        let mut parts: Vec<Value> = Vec::new();
                        for p in arr {
                            if let Some(t) = p.get("type").and_then(|v| v.as_str()) {
                                match t {
                                    "text" => {
                                        if let Some(txt) = p.get("text").and_then(|v| v.as_str()) {
                                            parts.push(json!({"type":"input_text","text": txt}));
                                        }
                                    }
                                    "image_url" => {
                                        if let Some(url) = p.get("image_url").and_then(|u| u.get("url")).and_then(|v| v.as_str()) {
                                            parts.push(json!({"type":"input_image","image_url": url}));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if !parts.is_empty() {
                            input.push(json!({"type":"message","role":"user","content": parts}));
                        }
                    }
                }
            }
            "assistant" => {
                // assistant content (if any)
                if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                    if !content.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": content}]
                        }));
                    }
                } else if let Some(arr) = m.get("content").and_then(|v| v.as_array()) {
                    // handle array content
                    for p in arr {
                        if let Some(txt) = p.get("text").and_then(|v| v.as_str()) {
                            input.push(json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": txt}]
                            }));
                        }
                    }
                }
                // tool calls -> function_call items
                if let Some(tool_calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let func = tc.get("function").unwrap_or(&Value::Null);
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                        // arguments should be JSON string; if already object, stringify
                        let args_str = if args.is_empty() {
                            if let Some(obj) = func.get("arguments") {
                                if obj.is_object() { obj.to_string() } else { "{}".to_string() }
                            } else { "{}".to_string() }
                        } else { args.to_string() };
                        input.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": args_str
                        }));
                    }
                }
            }
            "tool" => {
                let call_id = m.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
                let content_val = m.get("content").cloned().unwrap_or(Value::String(String::new()));
                let content_str = match &content_val {
                    Value::String(s) => s.clone(),
                    Value::Array(arr) => {
                        // image+text multimodal -> extract text part
                        let mut txt = String::new();
                        for p in arr {
                            if let Some(t) = p.get("text").and_then(|v| v.as_str()) { txt.push_str(t); }
                        }
                        if txt.is_empty() { content_val.to_string() } else { txt }
                    }
                    _ => content_val.to_string(),
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": content_str
                }));
            }
            "system" => {
                // already folded into instructions, skip
            }
            _ => {}
        }
    }
    (instructions, input)
}

pub fn build_responses_request_body(model_id: &str, instructions: &str, input: &[Value], tools: &[Value]) -> Value {
    json!({
        "model": model_id,
        "instructions": instructions,
        "input": input,
        "tools": tools,
        "stream": true,
        "store": false
    })
}

#[derive(Debug, Deserialize)]
pub struct ChatChunk {
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Delta {
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub tp: Option<String>,
    pub function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FunctionDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Usage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

// ── Responses API SSE types ─────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct ResponsesItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub call_id: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ResponsesCompleted {
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ResponsesEvent {
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta { delta: String, #[serde(default)] item_id: Option<String> },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone { text: String },
    #[serde(rename = "response.reasoning.delta")]
    ReasoningDelta { delta: String },
    #[serde(rename = "response.reasoning_text.delta")]
    ReasoningTextDelta { delta: String },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded { item: ResponsesItem },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone { item: ResponsesItem },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgsDelta {
        delta: String,
        #[serde(default)] item_id: Option<String>,
        #[serde(default)] output_index: Option<u32>,
        #[serde(default)] call_id: Option<String>,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgsDone { arguments: String },
    #[serde(rename = "response.completed")]
    Completed { response: ResponsesCompleted },
    #[serde(other)]
    Unknown,
}

/// Try to parse a single SSE `data:` payload as a ResponsesEvent.
/// Returns None for `[DONE]` or unparsable.
pub fn parse_responses_event(data: &str) -> Option<ResponsesEvent> {
    let trimmed = data.trim();
    if trimmed == "[DONE]" || trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<ResponsesEvent>(trimmed).ok()
}

/// Also try to parse chat chunk for fallback detection.
pub fn maybe_chat_chunk(data: &str) -> Option<ChatChunk> {
    serde_json::from_str::<ChatChunk>(data).ok()
}
