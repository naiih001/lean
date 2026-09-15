use serde_json::{json, Value};

#[derive(PartialEq)]
pub(crate) enum ContextKind {
    Focus,
    Continue,
}

pub(crate) fn classify_context_message(value: &Value) -> Option<ContextKind> {
    if value.get("role").and_then(|r| r.as_str()) != Some("system") {
        return None;
    }
    let content = value.get("content").and_then(|c| c.as_str())?;
    if content.starts_with("[Focus") || content.starts_with("[FOCUS") {
        Some(ContextKind::Focus)
    } else if content.starts_with("[Continue") || content.starts_with("[CONTINUATION") {
        Some(ContextKind::Continue)
    } else {
        None
    }
}

/// Drop injected focus/continue context so it cannot accumulate across steps.
pub(crate) fn prune_context_messages(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .filter(|m| classify_context_message(m).is_none())
        .cloned()
        .collect()
}

pub(crate) fn history_slice_for_api<'a>(history: &'a [Value]) -> &'a [Value] {
    let start = if history.len() > 20 {
        history.len() - 20
    } else {
        0
    };
    let slice = &history[start..];
    if slice
        .iter()
        .any(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
    {
        if let Some(first_non_tool) = slice
            .iter()
            .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("tool"))
        {
            if first_non_tool > 0
                && slice[..first_non_tool]
                    .iter()
                    .all(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
            {
                return &slice[first_non_tool..];
            }
        }
    }
    slice
}

pub(crate) fn drop_orphaned_tool_outputs(messages: &mut Vec<Value>) {
    let mut valid_ids = std::collections::HashSet::new();
    for m in messages.iter() {
        if m.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            if let Some(calls) = m.get("tool_calls").and_then(|v| v.as_array()) {
                for c in calls {
                    if let Some(id) = c.get("id").and_then(|v| v.as_str()) {
                        valid_ids.insert(id.to_string());
                    }
                }
            }
        }
    }
    messages.retain(|m| {
        if m.get("role").and_then(|r| r.as_str()) == Some("tool") {
            if let Some(id) = m.get("tool_call_id").and_then(|v| v.as_str()) {
                return valid_ids.contains(id);
            }
        }
        true
    });
}

pub(crate) const MAX_IMAGES_PER_TURN: usize = 5;

pub(crate) fn build_user_content(prompt: &str) -> Value {
    if !prompt.contains("<<IMAGE:") && !prompt.contains("<<IMAGE_URL:") {
        return Value::String(prompt.to_string());
    }
    let mut parts: Vec<Value> = Vec::new();
    let mut remaining = prompt;
    let mut count = 0usize;
    while let Some(start) = remaining.find("<<IMAGE") {
        let before = &remaining[..start];
        if !before.is_empty() {
            parts.push(json!({"type": "text", "text": before }));
        }
        if let Some(end) = remaining[start..].find(">>") {
            let marker = &remaining[start..start + end + 2];
            if marker.starts_with("<<IMAGE_URL:") {
                let url = marker
                    .trim_start_matches("<<IMAGE_URL:")
                    .trim_end_matches(">>");
                if count < MAX_IMAGES_PER_TURN {
                    parts.push(json!({"type": "image_url", "image_url": {"url": url}}));
                    count += 1;
                } else {
                    parts.push(json!({"type": "text", "text": format!("[image skipped: {} — max {} images/turn]", url, MAX_IMAGES_PER_TURN)}));
                }
            } else if marker.starts_with("<<IMAGE:") {
                let inner = marker.trim_start_matches("<<IMAGE:").trim_end_matches(">>");
                if let Some(colon) = inner.find(':') {
                    let mime = &inner[..colon];
                    let b64 = &inner[colon + 1..];
                    if count < MAX_IMAGES_PER_TURN {
                        parts.push(json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", mime, b64)}}));
                        count += 1;
                    } else {
                        parts.push(json!({"type": "text", "text": format!("[image skipped — max {} images/turn, {} omitted]", MAX_IMAGES_PER_TURN, mime)}));
                    }
                }
            }
            remaining = &remaining[start + end + 2..];
        } else {
            parts.push(json!({"type": "text", "text": remaining[start..].to_string()}));
            remaining = "";
            break;
        }
    }
    if !remaining.is_empty() {
        parts.push(json!({"type": "text", "text": remaining}));
    }
    if parts
        .iter()
        .any(|p| p.get("type").and_then(|t| t.as_str()) == Some("image_url"))
    {
        Value::Array(parts)
    } else {
        let txt: String = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
            .collect();
        Value::String(txt)
    }
}

/// Strip image content from messages for models that don't support vision.
pub(crate) fn strip_images_for_non_vision(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let mut out = m.clone();
            if let Some(content) = out.get("content") {
                if let Some(arr) = content.as_array() {
                    let has_image = arr
                        .iter()
                        .any(|p| p.get("type").and_then(|t| t.as_str()) == Some("image_url"));
                    if has_image {
                        let text_parts: Vec<String> = arr
                            .iter()
                            .filter_map(|p| {
                                let tp = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                match tp {
                                    "text" => p
                                        .get("text")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                    "image_url" => Some(
                                        "[image omitted — model does not support vision]"
                                            .to_string(),
                                    ),
                                    _ => None,
                                }
                            })
                            .collect();
                        out["content"] = json!(text_parts.join("\n"));
                    }
                }
            }
            out
        })
        .collect()
}
