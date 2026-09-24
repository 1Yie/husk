//! Normalized model — the only types that cross the provider boundary.
//!
//! * `arguments` fragments are concatenated, never parsed until `Done` or the next
//!   slot begins — JSON arrives split across chunks.
//! * `Done` fires exactly once per request, whatever the backend's sentinel.
//! * `Error` is a stream item, not stream termination — `SamplerActor` decides
//!   retry vs propagate.


use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A user-attached image on a `Role::User` message. Persisted as a path
/// reference — the file is staged inside the workspace (`.husk/attachments/`),
/// so it is visible to sandboxed tools AND durable for session replay.
/// Adapters materialize the bytes into a `data:` URL at wire-build time;
/// never inline the payload into the message itself (the session snapshot
/// would balloon to megabytes per image).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    /// Absolute path — the staged copy inside the workspace.
    pub path: PathBuf,
    /// MIME type (`image/png`, `image/jpeg`, …) — sniffed from the
    /// extension, good enough for `data:` URL framing.
    pub media_type: String,
}

impl ImageRef {
    /// Build a ref for `path` — `None` when the extension isn't an image
    /// type we can frame.
    pub fn for_path(path: PathBuf) -> Option<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())?;
        let media_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => return None,
        };
        Some(Self { path, media_type: media_type.into() })
    }

    /// Read the file and frame it as `data:<mime>;base64,<bytes>` — the
    /// shape every vision-capable wire protocol accepts. `None` when the
    /// file is gone or over the provider-size bound (20MB is the widest
    /// common cap — Anthropic 5MB is tighter but rare).
    pub fn data_url(&self) -> Option<String> {
        const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
        let meta = std::fs::metadata(&self.path).ok()?;
        if meta.len() > MAX_IMAGE_BYTES {
            return None;
        }
        let bytes = std::fs::read(&self.path).ok()?;
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{};base64,{}", self.media_type, b64))
    }
}

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
    /// Compacted-context memory note — a summary of earlier turns. Still
    /// `Role::System` internally (hidden on replay like the kernel
    /// prompt), but adapters MUST NOT fold it into `system`/`instructions`
    /// like a plain system row: it is historical context, not a live
    /// instruction. Each adapter emits it at user privilege instead.
    CompactedMemory,
    /// UI-only compaction card row (`ChatMessage::compaction`) — the
    /// persisted before/after accounting the frontend replays as a
    /// compaction card. Dropped from every provider wire body: it is
    /// render metadata, never context for the model.
    Compacted,
    /// UI-only plan card row (`ChatMessage::plan`) — the persisted
    /// `submit_plan` payload the frontend replays as the plan card. The
    /// model already has the plan in its own `submit_plan` tool call, so
    /// adapters drop it from the wire like [`NoticeKind::Compacted`].
    Plan,
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
    /// User-attached images — staged path references, materialized into
    /// `data:` URLs by each adapter at wire-build time. Only ever set on
    /// `Role::User` messages and only when the active model declares
    /// `"image"` in its `input` modalities; empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
    /// Reasoning/thinking trace of this assistant round — the persisted form
    /// of the live `思考过程` block, so a reopened session replays what the
    /// stream actually drew. Display-only: `reasoning` is a *separate*
    /// reasoning-model input, never a transcript field, so every adapter must
    /// keep it off the wire (`openai_compat` strips it like `is_error`).
    /// `None` on old snapshots and on rounds that produced no trace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Wall-clock duration (ms) of the activity this message records —
    /// `role=tool`: the call's dispatch-to-result span; `role=assistant`:
    /// the reasoning block's first→last delta span. Persisted so a replayed
    /// view can re-render the `思考过程 · 20s` / `工具调用 · 3s` elapsed
    /// labels the live stream drew. Display-only like `reasoning` — every
    /// adapter must keep it off the wire (`openai_compat` strips it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
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
    /// Attach this round's reasoning trace. Empty/whitespace traces are
    /// dropped rather than persisted as an empty thought block.
    pub fn with_reasoning(mut self, reasoning: Option<String>) -> Self {
        self.reasoning = reasoning.filter(|r| !r.trim().is_empty());
        self
    }

    /// Stamp the activity duration — reasoning span on assistant rows,
    /// dispatch span on tool results.
    pub fn with_duration_ms(mut self, ms: Option<i64>) -> Self {
        self.duration_ms = ms;
        self
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// Attach staged image refs — only call this when the active model
    /// declares `"image"` in its `input` modalities.
    pub fn with_images(mut self, images: Vec<ImageRef>) -> Self {
        self.images = images;
        self
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: None, ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// A user-facing system line the live stream emitted via
    /// `UiEvent::SystemMessage` — persisted so a reloaded view replays
    /// it exactly (vs `system()`, which is invisible internal context).
    pub fn notice(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::System), ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// Same, for a `UiEvent::Error` line — replays with the `⚠` prefix.
    pub fn notice_error(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::Error), ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// A `Role::User` instruction the UI never showed (injected by the
    /// engine, e.g. the synthesis nudge) — kept for the provider,
    /// skipped on replay.
    pub fn user_hidden(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::Hidden), ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// The compaction memory note — kept as `Role::System` so replay
    /// stays hidden, but tagged so adapters route it to user privilege
    /// instead of folding it into `system`/`instructions`.
    pub fn compacted_memory(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(text.into()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::CompactedMemory), ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// The persisted compaction card — a JSON payload (`before_tokens`,
    /// `after_tokens`, `removed_messages`, `manual`, `note`) the frontend
    /// renders as the compaction card on replay. `NoticeKind::Compacted`
    /// rows are dropped by every adapter, so the note here never costs wire
    /// tokens; the model sees the summary through the `compacted_memory`
    /// row. `manual` keeps the user-run `/compact` card a standalone block
    /// on replay instead of folding it into the previous turn.
    pub fn compaction(before_tokens: u32, after_tokens: u32, removed_messages: u32, manual: bool, note: &str) -> Self {
        let payload = serde_json::json!({
            "before_tokens": before_tokens,
            "after_tokens": after_tokens,
            "removed_messages": removed_messages,
            "manual": manual,
            "note": note,
        });
        Self { role: Role::System, content: Some(payload.to_string()), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::Compacted), ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// The persisted plan card — the `submit_plan` payload as a JSON string
    /// (`summary`, `steps`, `verification`, `risks`) the frontend renders as
    /// the plan card on replay. `NoticeKind::Plan` rows are dropped by every
    /// adapter — the model sees the plan through its own `submit_plan` call.
    pub fn plan(payload: String) -> Self {
        Self { role: Role::System, content: Some(payload), tool_calls: None, tool_call_id: None, is_error: None, notice: Some(NoticeKind::Plan), ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    pub fn tool_result(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: Some(text.into()), tool_calls: None, tool_call_id: Some(call_id.into()), is_error: None, notice: None, ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
    /// Failed tool result — same wire shape, plus the persisted `is_error`
    /// flag the UI replays into the red capsule state.
    pub fn tool_result_err(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: Some(text.into()), tool_calls: None, tool_call_id: Some(call_id.into()), is_error: Some(true), notice: None, ts: Some(now_ms()), images: Vec::new(), reasoning: None, duration_ms: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Canonical invocation id — provider-issued when the wire has one
    /// (`call_…`, `toolu_…`), `call_*`-synthesized by the assembler when it
    /// doesn't (Gemini `functionCall` carries no id at all). Never empty
    /// once a call reaches the engine. Adapters replay it through their
    /// own correlation field (`call_id`, `tool_use_id`); name-keyed wires
    /// (Gemini `functionResponse`) resolve the tool name by looking this
    /// id up against the earlier assistant `tool_calls`.
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
    /// Tool-call fragment — id/name arrive once, arguments stream as JSON shards under
    /// `args_delta`.
    ///
    /// `slot` is an opaque merge key for which invocation a delta belongs to, not a
    /// positional index: OpenAI sends `tool_calls[i].index`, Responses a sparse
    /// `output_index`, Anthropic a content-block `index`, Gemini a counter. The
    /// assembler only needs deltas of one invocation to share a slot, and slots to
    /// sort in call order.
    ToolCallDelta {
        slot: usize,
        id: Option<String>,
        name: Option<String>,
        args_delta: String,
    },
    /// Terminal chunk; carries usage when the backend reports it.
    Done {
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
        /// Prompt tokens served from the provider's prompt cache —
        /// `prompt - cached` is the fresh/billed portion.
        cached_tokens: Option<u32>,
    },
    Error(String),
}

/// Assemble a completed tool call list from a chunk stream — used by the
/// sampler and engine. Fragments for the same `slot` merge by concat.
///
/// Slots are a map, not a Vec: the Responses API's `output_index` counts
/// **every** output item (reasoning=0, message=1, first call=2…), so call
/// indices are sparse. Gap-filling a Vec materialized an empty-name call
/// per skipped index — the engine then logged "已跳过一个格式异常的空工具
/// 调用" once per reasoning/message item before each real call.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    calls: std::collections::BTreeMap<usize, ToolCall>,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one chunk. Returns `true` when the chunk was a tool delta.
    pub fn feed(&mut self, chunk: &StreamChunk) -> bool {
        if let StreamChunk::ToolCallDelta { slot, id, name, args_delta } = chunk {
            let call = self.calls.entry(*slot).or_insert_with(|| ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
            if let Some(id) = id {
                // id arrives once — assign, never append (Responses repeats
                // call_id on argument deltas; appending corrupts it).
                if call.id.is_empty() {
                    call.id = id.clone();
                }
            }
            if let Some(name) = name {
                // name likewise arrives once — assign; appending produced
                // `smart_readsmart_read…` when the args delta re-sent it.
                if call.name.is_empty() {
                    call.name = name.clone();
                }
            }
            call.arguments.push_str(args_delta);
            true
        } else {
            false
        }
    }

    /// Completed calls in `slot` order — sparse slots collapse; no
    /// empty slots survive.
    ///
    /// Backfills a synthesized `id` on any call the provider never named:
    /// some Responses-API backends (deepseek) only expose `call_id` on
    /// `output_item.done` — or not at all — and persisting `id: ""`
    /// replays next turn as `function_call.call_id: ""`, which strict
    /// upstreams reject with `400 invalid_request_error`.
    pub fn finish(mut self) -> Vec<ToolCall> {
        static SYNTH_SEQ: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        for call in self.calls.values_mut() {
            if call.id.is_empty() {
                call.id = format!(
                    "call_{:x}_{}",
                    now_ms(),
                    SYNTH_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
            }
        }
        self.calls.into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_card_roundtrips_the_frontend_payload() {
        // The card row is persisted verbatim and parsed by the webview —
        // field names are part of the frontend contract.
        let m = ChatMessage::compaction(126_995, 64_019, 77, true, "merged summary");
        assert_eq!(m.role, Role::System);
        assert_eq!(m.notice, Some(NoticeKind::Compacted));
        let payload: serde_json::Value =
            serde_json::from_str(m.content.as_deref().unwrap()).unwrap();
        assert_eq!(payload["before_tokens"], 126_995);
        assert_eq!(payload["after_tokens"], 64_019);
        assert_eq!(payload["removed_messages"], 77);
        assert_eq!(payload["manual"], true);
        assert_eq!(payload["note"], "merged summary");

        // Notice kind serializes to the snake_case string the TS union
        // matches on.
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"compacted\""), "{j}");
    }

    /// The reasoning trace is what a reopened session replays as its
    /// 思考过程 block, so it must survive the JSONL snapshot exactly — and a
    /// trace-free row must not grow the file.
    #[test]
    fn reasoning_roundtrips_and_old_snapshots_default() {
        let m = ChatMessage::assistant("答案").with_reasoning(Some("先想一下".into()));
        let j = serde_json::to_string(&m).unwrap();
        let back: ChatMessage = serde_json::from_str(&j).unwrap();
        assert_eq!(back.reasoning.as_deref(), Some("先想一下"));

        // A row written before the field existed still loads.
        let old = r#"{"role":"assistant","content":"hi"}"#;
        let back: ChatMessage = serde_json::from_str(old).unwrap();
        assert_eq!(back.reasoning, None);

        // No trace → no key at all.
        let plain = serde_json::to_string(&ChatMessage::assistant("x")).unwrap();
        assert!(!plain.contains("reasoning"), "{plain}");
    }

    #[test]
    fn assembler_collapses_sparse_output_indices() {
        // Responses `output_index` counts reasoning/message items too —
        // a call at index 2 must not materialize empty slots 0 and 1.
        let mut a = ToolCallAssembler::new();
        assert!(a.feed(&StreamChunk::ToolCallDelta {
            slot: 2,
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
