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
    /// `None` serializes as `""` on the wire — some OpenAI-compat backends
    /// (devin verified) treat a literal `content:null` on an assistant
    /// tool-call message as malformed and return an empty stream instead of
    /// an error, which surfaces as "the agent went quiet after a tool".
    #[serde(serialize_with = "content_as_string")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

fn content_as_string<S: serde::Serializer>(
    c: &Option<String>,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(c.as_deref().unwrap_or(""))
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

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON assembled incrementally from `args_delta` fragments.
    pub arguments: String,
}

// OpenAI wire shape: `{"id":…,"type":"function","function":{"name":…,
// "arguments":"{…}"}}` — NOT our flat `{id,name,arguments}`. A tool-call
// round-trip serialized flat makes the next request's assistant message
// malformed and the model stops responding after the first tool call
// (verified against devin/swe-2).
impl Serialize for ToolCall {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("ToolCall", 3)?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("type", "function")?;
        st.serialize_field("function", &serde_json::json!({
            "name": self.name,
            "arguments": self.arguments,
        }))?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for ToolCall {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Accept both the OpenAI shape and the flat shape on the way in.
        #[derive(Deserialize)]
        struct Flat { id: String, name: String, arguments: String }
        #[derive(Deserialize)]
        struct Func { name: String, arguments: String }
        #[derive(Deserialize)]
        struct OpenAi { id: String, function: Func }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Any { Oa(OpenAi), Fl(Flat) }
        match Any::deserialize(d)? {
            Any::Oa(o) => Ok(ToolCall { id: o.id, name: o.function.name, arguments: o.function.arguments }),
            Any::Fl(f) => Ok(ToolCall { id: f.id, name: f.name, arguments: f.arguments }),
        }
    }
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
                // id arrives once — assign, never append (Responses repeats
                // call_id on argument deltas; appending corrupts it).
                if slot.id.is_empty() {
                    slot.id = id.clone();
                }
            }
            if let Some(name) = name {
                // name likewise arrives once — assign; appending produced
                // `smart_readsmart_read…` when the args delta re-sent it.
                if slot.name.is_empty() {
                    slot.name = name.clone();
                }
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
