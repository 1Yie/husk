//! Normalized model — the ONLY types that cross the provider boundary.
//!
//! Rules (llm-provider-layer.md §types):
//! * `arguments` fragments are **concatenated**, never parsed until `Done`
//!   or the next `index` begins — JSON arrives split across chunks.
//! * `Done` fires exactly once per request, even on `[DONE]`-sentinel vs.
//!   clean-close differences between backends.
//! * `Error` is a stream item, not stream termination — SamplerActor decides
//!   retry vs. propagate.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: Some(text.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn tool_result(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: Some(text.into()), tool_calls: None, tool_call_id: Some(call_id.into()) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON assembled incrementally from `args_delta` fragments.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    /// Reasoning trace (DeepSeek-R1 `reasoning_content`, Claude thinking,
    /// Grok reasoning).
    ReasoningDelta(String),
    /// User-visible text delta.
    ContentDelta(String),
    /// Tool-call fragment — id/name arrive once, arguments stream as JSON
    /// shards under `args_delta`.
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        args_delta: String,
    },
    /// Terminal chunk; carries usage when the backend reports it.
    Done {
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    },
    Error(String),
}

/// Assemble a completed tool call list from a chunk stream — used by the
/// sampler and engine. Fragments for the same `index` merge by concat.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    slots: Vec<ToolCall>,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one chunk. Returns `true` when the chunk was a tool delta.
    pub fn feed(&mut self, chunk: &StreamChunk) -> bool {
        if let StreamChunk::ToolCallDelta { index, id, name, args_delta } = chunk {
            while self.slots.len() <= *index {
                self.slots.push(ToolCall { id: String::new(), name: String::new(), arguments: String::new() });
            }
            let slot = &mut self.slots[*index];
            if let Some(id) = id {
                slot.id.push_str(id);
            }
            if let Some(name) = name {
                slot.name.push_str(name);
            }
            slot.arguments.push_str(args_delta);
            true
        } else {
            false
        }
    }

    /// Completed calls in `index` order.
    pub fn finish(self) -> Vec<ToolCall> {
        self.slots
    }
}
