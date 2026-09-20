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

/// How a persisted message replays in the UI. On `Role::System` entries,
/// `Some` marks a line the live stream actually showed (`SystemMessage` /
/// `Error` events) while `None` is internal context (system prompt,
/// compaction note, hook injection) — hidden. `Hidden` marks a message
/// the provider must see but the UI never rendered (injected
/// instructions) — without it a reloaded view invents a user bubble the
/// user never typed. Adapters strip this before the wire like
/// [`ChatMessage::is_error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// Plain system line — e.g. "已被用户中断", a `/` command reply.
    System,
    /// `⚠`-prefixed error line — what the `UiEvent::Error` arm renders.
    Error,
    /// Persisted for the provider but never drawn — injected
    /// instructions like the tool-limit nudge.
    Hidden,
}

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
    /// Replay-only metadata: this tool result carried a failure (veto,
    /// deny, dispatch error, write failure, …). Persisted in the session
    /// snapshot so a reloaded view can re-render the failed capsule —
    /// without it every tool call replays as `ok`. Adapters that forward
    /// `ChatMessage` raw (`openai_compat`) must strip it before the wire;
    /// strict backends reject unknown fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Display class for replay — see [`NoticeKind`]. `None` renders by
    /// role (system = internal/hidden, others = their normal item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<NoticeKind>,
    /// Creation time (epoch ms) — persisted so a replayed view can draw
    /// the same `—— time ——` turn divider the live stream showed. `None`
    /// on old snapshots → no divider for those messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

/// Wall-clock epoch millis — stamps every message at construction.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn content_as_string<S: serde::Serializer>(
    c: &Option<String>,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(c.as_deref().unwrap_or(""))
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: Some(now_ms()) }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: Some(now_ms()) }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: Some(now_ms()) }
    }
    /// A user-facing system line the live stream emitted via
    /// `UiEvent::SystemMessage` — persisted so a reloaded view replays
    /// it exactly (vs `system()`, which is invisible internal context).
    pub fn notice(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::System), ts: Some(now_ms()) }
    }
    /// Same, for a `UiEvent::Error` line — replays with the `⚠` prefix.
    pub fn notice_error(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::Error), ts: Some(now_ms()) }
    }
    /// A `Role::User` instruction the UI never showed (injected by the
    /// engine, e.g. the synthesis nudge) — kept for the provider,
    /// skipped on replay.
    pub fn user_hidden(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::Hidden), ts: Some(now_ms()) }
    }
    pub fn tool_result(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: Some(text.into()), tool_calls: None, tool_call_id: Some(call_id.into()), is_error: None, notice: None, ts: Some(now_ms()) }
    }
    /// Failed tool result — same wire shape, plus the persisted `is_error`
    /// flag the UI replays into the red capsule state.
    pub fn tool_result_err(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: Some(text.into()), tool_calls: None, tool_call_id: Some(call_id.into()), is_error: Some(true), notice: None, ts: Some(now_ms()) }
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
///
/// Slots are a map, not a Vec: the Responses API's `output_index` counts
/// **every** output item (reasoning=0, message=1, first call=2…), so call
/// indices are sparse. Gap-filling a Vec materialized an empty-name call
/// per skipped index — the engine then logged "已跳过一个格式异常的空工具
/// 调用" once per reasoning/message item before each real call.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    slots: std::collections::BTreeMap<usize, ToolCall>,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one chunk. Returns `true` when the chunk was a tool delta.
    pub fn feed(&mut self, chunk: &StreamChunk) -> bool {
        if let StreamChunk::ToolCallDelta { index, id, name, args_delta } = chunk {
            let slot = self.slots.entry(*index).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
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

    /// Completed calls in `index` order — sparse indices collapse; no
    /// empty slots survive.
    pub fn finish(self) -> Vec<ToolCall> {
        self.slots.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembler_collapses_sparse_output_indices() {
        // Responses `output_index` counts reasoning/message items too —
        // a call at index 2 must not materialize empty slots 0 and 1.
        let mut a = ToolCallAssembler::new();
        assert!(a.feed(&StreamChunk::ToolCallDelta {
            index: 2,
            id: Some("c1".into()),
            name: Some("bash".into()),
            args_delta: "{}".into(),
        }));
        let calls = a.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }

    #[test]
    fn notice_roundtrips_and_old_snapshots_default() {
        // New format serializes `notice`; old snapshots lack it → `None`.
        let m = ChatMessage::notice("已被用户中断");
        let j = serde_json::to_string(&m).unwrap();
        let back: ChatMessage = serde_json::from_str(&j).unwrap();
        assert_eq!(back.notice, Some(NoticeKind::System));
        let m = ChatMessage::notice_error("upstream error");
        let j = serde_json::to_string(&m).unwrap();
        let back: ChatMessage = serde_json::from_str(&j).unwrap();
        assert_eq!(back.notice, Some(NoticeKind::Error));
        let old = r#"{"role":"system","content":"a context note"}"#;
        let back: ChatMessage = serde_json::from_str(old).unwrap();
        assert_eq!(back.notice, None);
    }
}
