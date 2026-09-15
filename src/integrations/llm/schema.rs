use serde_json::{json, Value};

pub fn native_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read",
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
                "name": "write",
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
                "name": "edit",
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
                "name": "grep",
                "description": "Search for pattern in files (like grep -r)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Regex or string to search"},
                        "path": {"type": "string", "description": "Directory or file to search (default cwd)"}
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "find",
                "description": "Find files by name pattern",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Glob pattern e.g. *.rs"},
                        "path": {"type": "string", "description": "Directory to search (default cwd)"}
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "ls",
                "description": "List files in a directory",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory to list (default cwd)"}
                    },
                    "required": []
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
                "name": "web_fetch",
                "description": "Fetch a web page by URL",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "URL to fetch"}
                    },
                    "required": ["url"]
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
                "name": "read_agent",
                "description": "Load an agent definition by name",
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
                "name": "subagent",
                "description": "Spawn a subagent (scout, researcher, worker) with a task. Requires a unique human label for tracking (shown first in popup, e.g. 'research-auth'). Auto-suffixed if duplicate.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent": {"type": "string", "description": "Agent to spawn (e.g. scout, researcher)"},
                        "task": {"type": "string", "description": "Task/prompt for subagent"},
                        "name": {"type": "string", "description": "Required unique label for tracking (e.g. 'crawler-1'). Auto-suffixed if taken"}
                    },
                    "required": ["agent", "task", "name"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "subagents_list",
                "description": "List live background subagents still running (id, agent, task, status, elapsed)",
                "parameters": {
                    "type": "object",
                    "properties": {}
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
        json!({
            "type": "function",
            "function": {
                "name": "ask_user",
                "description": "Ask the user one or more clarifying questions with concrete options. In PLAN mode, MANDATORY in Phases 2-3 before any mutation (except .hermes/plans) to confirm scope/approach and get explicit '✓ Proceed as proposed'; iterate until 100% sure. In regular mode, use when genuinely ambiguous (unclear target, scope, preference) and you can't discover the answer; otherwise bias toward doing. The UI asks one question at a time; every question also offers an 'Other…' free-text row. In plan mode the final gating question MUST contain an option exactly labeled '✓ Proceed as proposed'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 4,
                            "description": "Questions to ask, in order",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "header": {"type": "string", "description": "Short label, <= 24 chars"},
                                    "question": {"type": "string", "description": "The question to ask"},
                                    "multi_select": {"type": "boolean", "default": false, "description": "Allow multiple options to be selected"},
                                    "options": {
                                        "type": "array",
                                        "minItems": 2,
                                        "maxItems": 6,
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "label": {"type": "string", "description": "Option label, <= 60 chars"},
                                                "description": {"type": "string", "description": "Optional one-line explanation"}
                                            },
                                            "required": ["label"]
                                        }
                                    }
                                },
                                "required": ["question", "options"]
                            }
                        }
                    },
                    "required": ["questions"]
                }
            }
        }),
    ]
}

pub async fn tool_definitions() -> Vec<Value> {
    let mut v = native_tool_definitions();
    let mcp_tools = crate::integrations::mcp::mcp_tool_definitions().await;
    v.extend(mcp_tools);
    v
}

pub fn responses_tool_definitions_from(chat_defs: &[Value]) -> Vec<Value> {
    chat_defs
        .iter()
        .map(|v| {
            let f = &v["function"];
            if f.is_null() || f == &Value::Null {
                return v.clone();
            }
            let name = f
                .get("name")
                .cloned()
                .unwrap_or(Value::String(String::new()));
            let desc = f
                .get("description")
                .cloned()
                .unwrap_or(Value::String(String::new()));
            let params = f
                .get("parameters")
                .cloned()
                .unwrap_or(json!({"type":"object","properties":{}}));
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
    let mcp_tools = crate::integrations::mcp::mcp_tool_definitions().await;
    v.extend(responses_tool_definitions_from(&mcp_tools));
    v
}

pub async fn tool_definitions_for(mode: &crate::integrations::models::ApiMode) -> Vec<Value> {
    match mode {
        crate::integrations::models::ApiMode::Responses => responses_tool_definitions().await,
        _ => tool_definitions().await,
    }
}

pub fn chat_messages_to_responses_input(
    messages: &[Value],
    system_prompt: &str,
) -> (String, Vec<Value>) {
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
                                        if let Some(url) = p
                                            .get("image_url")
                                            .and_then(|u| u.get("url"))
                                            .and_then(|v| v.as_str())
                                        {
                                            parts.push(
                                                json!({"type":"input_image","image_url": url}),
                                            );
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
                if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                    if !content.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": content}]
                        }));
                    }
                } else if let Some(arr) = m.get("content").and_then(|v| v.as_array()) {
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
                if let Some(tool_calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let func = tc.get("function").unwrap_or(&Value::Null);
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                        let args_str = if args.is_empty() {
                            if let Some(obj) = func.get("arguments") {
                                if obj.is_object() {
                                    obj.to_string()
                                } else {
                                    "{}".to_string()
                                }
                            } else {
                                "{}".to_string()
                            }
                        } else {
                            args.to_string()
                        };
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
                let content_val = m
                    .get("content")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                let content_str = match &content_val {
                    Value::String(s) => s.clone(),
                    Value::Array(arr) => {
                        let mut txt = String::new();
                        for p in arr {
                            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                                txt.push_str(t);
                            }
                        }
                        if txt.is_empty() {
                            content_val.to_string()
                        } else {
                            txt
                        }
                    }
                    _ => content_val.to_string(),
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": content_str
                }));
            }
            "system" => {}
            _ => {}
        }
    }
    (instructions, input)
}

pub fn build_responses_request_body(
    model_id: &str,
    instructions: &str,
    input: &[Value],
    tools: &[Value],
) -> Value {
    json!({
        "model": model_id,
        "instructions": instructions,
        "input": input,
        "tools": tools,
        "stream": true,
        "store": false
    })
}
