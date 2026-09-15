use serde::Deserialize;

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
    OutputTextDelta {
        delta: String,
        #[serde(default)]
        item_id: Option<String>,
    },
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
        #[serde(default)]
        item_id: Option<String>,
        #[serde(default)]
        output_index: Option<u32>,
        #[serde(default)]
        call_id: Option<String>,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgsDone {
        arguments: String,
        #[serde(default)]
        item_id: Option<String>,
        #[serde(default)]
        output_index: Option<u32>,
        #[serde(default)]
        call_id: Option<String>,
    },
    #[serde(rename = "response.completed")]
    Completed { response: ResponsesCompleted },
    #[serde(other)]
    Unknown,
}

pub fn parse_responses_event(data: &str) -> Option<ResponsesEvent> {
    let trimmed = data.trim();
    if trimmed == "[DONE]" || trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<ResponsesEvent>(trimmed).ok()
}

pub fn maybe_chat_chunk(data: &str) -> Option<ChatChunk> {
    serde_json::from_str::<ChatChunk>(data).ok()
}
