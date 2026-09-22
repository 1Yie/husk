//! OpenAI-compatible adapter — one `GenericOpenAiProvider` covers every
//! `/v1/chat/completions` SSE dialect: OpenAI, xAI, DeepSeek,
//! Ollama(`/v1`), vLLM, LM Studio.
//!
//! Wire mapping (llm-provider-layer.md):
//!
//! | Wire field                    | → StreamChunk                                  |
//! |-------------------------------|------------------------------------------------|
//! | `delta.reasoning_content`     | `ReasoningDelta`                               |
//! | `delta.content`               | `ContentDelta`                                 |
//! | `delta.tool_calls[i]`         | `ToolCallDelta{index, id?, name?, args_delta}` |
//! | `data == "[DONE]"`            | `Done{None, None}`                             |
//! | `usage` block (final chunk)   | merged into `Done`                             |
//! | non-2xx on `send()`           | `Err` before stream starts, body included      |

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::provider::{BoxStream, LlmProvider, ModelParams};
use crate::transport::{DoneGuard, Transport};
use crate::types::{ChatMessage, Role, StreamChunk};

/// One provider for every OpenAI-shaped backend. `api_key` is the *resolved*
/// secret — config's `env:`/`keyring:` indirection happens in `factory`.
pub struct GenericOpenAiProvider {
    transport: Transport,
    base_url: String,
    api_key: String,
    compat: Option<crate::config::ProviderCompat>,
}

impl GenericOpenAiProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            transport: Transport::new()?,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            compat: None,
        })
    }

    pub fn with_compat(mut self, compat: crate::config::ProviderCompat) -> Self {
        self.compat = Some(compat);
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.transport = self.transport.with_header(key, value);
        self
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

// ---- wire types (vendor-shaped, never leave this module) ----

#[derive(Debug, Deserialize)]
struct WireChunk {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    #[serde(default)]
    delta: WireDelta,
}

#[derive(Debug, Default, Deserialize)]
struct WireDelta {
    #[serde(default)]
    reasoning_content: Option<String>,
    /// Grok/xAI emits `reasoning` instead of `reasoning_content` on some models.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Debug, Deserialize)]
struct WireToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireFunction>,
}

#[derive(Debug, Deserialize)]
struct WireFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize)]
pub(crate) struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    /// OpenAI shape: `prompt_tokens_details.cached_tokens`.
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    /// DeepSeek-style flat fields.
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u32>,
    #[serde(default)]
    prompt_cache_miss_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize)]
pub(crate) struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

impl WireUsage {
    fn cached(&self) -> Option<u32> {
        self.prompt_tokens_details
            .and_then(|d| d.cached_tokens)
            .or(self.prompt_cache_hit_tokens)
    }
}

/// Map one SSE `data:` payload into zero-or-more normalized chunks.
/// Returns `None` for payloads that carry no model-visible information
/// (e.g. role-only first delta, ping frames).
fn map_data(data: &str, pending_usage: &mut Option<WireUsage>) -> Vec<StreamChunk> {
    if data.trim() == "[DONE]" {
        let usage = pending_usage.take();
        return vec![StreamChunk::Done {
            prompt_tokens: usage.and_then(|u| u.prompt_tokens),
            completion_tokens: usage.and_then(|u| u.completion_tokens),
            cached_tokens: usage.and_then(|u| u.cached()),
        }];
    }

    let chunk: WireChunk = match serde_json::from_str(data) {
        Ok(c) => c,
        Err(e) => {
            return vec![StreamChunk::Error(format!(
                "malformed SSE JSON: {e}; payload: {}",
                &data[..data.len().min(120)]
            ))]
        }
    };

    // Some backends send usage in a final standalone chunk (choices empty).
    if let Some(u) = chunk.usage {
        *pending_usage = Some(u);
        if chunk.choices.is_empty() {
            return Vec::new();
        }
    }

    let mut out = Vec::new();
    for choice in &chunk.choices {
        let d = &choice.delta;
        if let Some(r) = d.reasoning_content.as_ref().or(d.reasoning.as_ref()) {
            if !r.is_empty() {
                out.push(StreamChunk::ReasoningDelta(r.clone()));
            }
        }
        if let Some(c) = &d.content {
            if !c.is_empty() {
                out.push(StreamChunk::ContentDelta(c.clone()));
            }
        }
        if let Some(tcs) = &d.tool_calls {
            for tc in tcs {
                let (name, args) = tc
                    .function
                    .as_ref()
                    .map(|f| (f.name.clone(), f.arguments.clone().unwrap_or_default()))
                    .unwrap_or((None, String::new()));
                out.push(StreamChunk::ToolCallDelta {
                    slot: tc.index,
                    id: tc.id.clone(),
                    name,
                    args_delta: args,
                });
            }
        }
    }
    out
}

/// Test-only handle on the wire→normalized mapper (unit-tested without HTTP).
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn map_data_for_test(
    data: &str,
    pending_usage: &mut Option<WireUsage>,
) -> Vec<StreamChunk> {
    map_data(data, pending_usage)
}

/// Public test shim so integration tests (tests/) can drive the same mapper
/// without a live HTTP endpoint. Not compiled into release builds for
/// downstream crates in test mode only — `#[doc(hidden)]` keeps it out of
/// the API surface.
#[doc(hidden)]
pub fn test_map_data(
    data: &str,
    pending_usage: &mut Option<serde_json::Value>,
) -> Vec<StreamChunk> {
    // Bridge: internal map_data uses the concrete WireUsage; tests just need
    // "did usage fold into Done" — emulate by stashing a marker.
    let mut internal: Option<WireUsage> = pending_usage.as_ref().and_then(|v| {
        serde_json::from_value::<WireUsage>(v.clone()).ok()
    });
    let out = map_data(data, &mut internal);
    *pending_usage = internal.map(|u| serde_json::to_value(u).unwrap_or_default());
    out
}

/// Serialize one `ChatMessage` to its wire `Value` — replay-only fields
/// (`is_error`, `notice`, `ts`, `images`, `reasoning`) are stripped; a user message with
/// image refs becomes a content-parts array (text first, `image_url` parts
/// after; unresolvable refs degrade to a text note).
fn serialize_message(m: &ChatMessage) -> serde_json::Value {
    let mut v = serde_json::to_value(m).unwrap_or_else(|_| {
        json!({ "role": "user", "content": "" })
    });
    if let Some(obj) = v.as_object_mut() {
        obj.remove("is_error"); obj.remove("notice"); obj.remove("ts");
        // The stored reasoning trace is ours to replay, not the provider's to
        // read: strict backends reject unknown fields outright.
        obj.remove("reasoning");
        if !m.images.is_empty() && matches!(m.role, Role::User) {
            let mut parts = vec![json!({
                "type": "text",
                "text": m.content.clone().unwrap_or_default(),
            })];
            for img in &m.images {
                match img.data_url() {
                    Some(url) => parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": url },
                    })),
                    None => parts.push(json!({
                        "type": "text",
                        "text": format!("(image unavailable: {})", img.path.display()),
                    })),
                }
            }
            obj.insert("content".into(), json!(parts));
        }
        obj.remove("images");
    }
    v
}

/// `ChatMessage` list → wire `messages` with a tool-pairing integrity pass.
///
/// Strict OpenAI-compat backends (deepseek verified) reject the whole
/// request — `400 "No tool output found for tool call …"` — when history
/// carries either side of a broken pair:
///
/// * a `tool_calls` entry with no matching `role:"tool"` output (a skipped
///   malformed call, an interrupted turn, a compacted-away output, a
///   poisoned snapshot), or
/// * a `tool` message whose `tool_call_id` never appeared on an earlier
///   `tool_calls` entry (orphan output).
///
/// Repair rules, applied in order:
/// 1. `tool_calls` entries with an empty `id` get a synthesized one (the
///    assembler backfills live, but old snapshots can persist `""`).
/// 2. A non-`tool` message arriving while outputs are still expected flushes
///    synthesized placeholder outputs — they must sit immediately after
///    their assistant row, before whatever comes next.
/// 3. `tool` rows with an unseen `tool_call_id` are dropped; a blank
///    `tool_call_id` pairs positionally with the earliest outstanding call.
#[cfg(test)]
fn build_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    build_messages_dev(messages, &crate::config::ProviderCompat::default())
}

/// Same pass, plus the `compat`-driven shapes pi documents for partial
/// OpenAI compatibility:
///
/// * `requiresToolResultName` — add `name` to `role:"tool"` rows (backends
///   that expect the function name alongside the id).
/// * `requiresAssistantAfterToolResult` — insert an assistant message between
///   a run of tool results and the following user message.
/// * `requiresReasoningContentOnAssistantMessages` — replay `reasoning_content: ""`
///   on assistant rows so a backend that requires the key doesn't 400.
fn build_messages_dev(
    messages: &[ChatMessage],
    compat: &crate::config::ProviderCompat,
) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
    // Call ids still awaiting their `tool` output, in call order.
    let mut expected: Vec<String> = Vec::new();
    // id → function name, for `requiresToolResultName`.
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut synth_seq = 0usize;
    let result_name = compat.requires_tool_result_name.unwrap_or(false);

    // Missing outputs must be emitted directly after their assistant row —
    // any non-tool message boundary flushes them first.
    macro_rules! flush_missing {
        () => {
            for id in expected.drain(..) {
                let mut row = json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": "(tool result missing — the call was interrupted or dropped from history)",
                });
                if result_name {
                    row["name"] = json!(names.get(&id).cloned().unwrap_or_default());
                }
                out.push(row);
            }
        };
    }

    for m in messages {
        let mut v = serialize_message(m);
        if m.role == Role::System && compat.developer_role() {
            // pi: `supportsDeveloperRole: true` means the endpoint understands
            // the newer `developer` role. Default is `system` — which every
            // OpenAI-compatible shim accepts.
            v["role"] = json!("developer");
        }
        match m.role {
            Role::Assistant => {
                flush_missing!();
                // Rewrite empty call ids in the emitted message so the wire
                // and the pairing ledger agree.
                if let Some(calls) = v.get_mut("tool_calls").and_then(|c| c.as_array_mut()) {
                    for c in calls.iter_mut() {
                        let id = c.get("id").and_then(|i| i.as_str()).unwrap_or_default();
                        let fname = c
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if id.is_empty() {
                            synth_seq += 1;
                            let id = format!("call_synth_{synth_seq}");
                            c["id"] = json!(id);
                            names.insert(id.clone(), fname);
                            expected.push(id);
                        } else {
                            names.insert(id.to_string(), fname);
                            expected.push(id.to_string());
                        }
                    }
                }
                if compat.requires_reasoning_content_on_assistant_messages == Some(true) {
                    // A backend that insists on the key present will reject an
                    // assistant row that omits it — an empty string is the
                    // shape it expects when there is nothing to replay.
                    v["reasoning_content"] = json!("");
                }
                out.push(v);
            }
            Role::Tool => {
                let mut call_id = m.tool_call_id.clone().unwrap_or_default();
                if call_id.is_empty() {
                    // Blank id pairs positionally with the earliest
                    // outstanding call (engine dispatched in order).
                    if expected.is_empty() {
                        continue; // orphan — no call to pair with
                    }
                    call_id = expected.remove(0);
                    v["tool_call_id"] = json!(call_id.clone());
                    if result_name {
                        v["name"] = json!(names.get(&call_id).cloned().unwrap_or_default());
                    }
                    out.push(v);
                } else if let Some(pos) = expected.iter().position(|id| *id == call_id) {
                    expected.remove(pos);
                    if result_name {
                        v["name"] = json!(names.get(&call_id).cloned().unwrap_or_default());
                    }
                    out.push(v);
                }
                // else: orphan tool output — drop it.
            }
            Role::User => {
                // Some backends require a non-tool message to be preceded by an
                // assistant turn once tool results have been replayed.
                if compat.requires_assistant_after_tool_result == Some(true) {
                    if let Some(last) = out.last() {
                        if last.get("role").and_then(|r| r.as_str()) == Some("tool") {
                            out.push(json!({ "role": "assistant", "content": "" }));
                        }
                    }
                }
                flush_missing!();
                out.push(v);
            }
            _ => {
                flush_missing!();
                out.push(v);
            }
        }
    }
    // Trailing dangling calls — the last assistant row ended the history
    // mid-tools (cancelled turn, crash before persist).
    flush_missing!();
    out
}

/// Write the thinking directive in the dialect the provider declares
/// (`compat.thinkingFormat`). `openai` (the default) sends `reasoning_effort`;
/// the others express the same intent through their provider-specific field.
///
/// Dialects not yet mapped here fall back to `reasoning_effort`, which is
/// what every OpenAI-compatible shim that isn't one of the listed dialects
/// accepts.
fn apply_thinking(body: &mut serde_json::Value, compat: &crate::config::ProviderCompat, effort: &str) {
    match compat.thinking_format_name() {
        // `reasoning: { effort }` — OpenRouter's shape.
        "openrouter" => {
            body["reasoning"] = json!({ "effort": effort });
        }
        // `reasoning: { enabled }` — Together; `reasoning_effort` too when the
        // provider also declares support for it.
        "together" => {
            body["reasoning"] = json!({ "enabled": effort != "none" && effort != "off" });
            if compat.supports_reasoning_effort == Some(true) {
                body["reasoning_effort"] = json!(effort);
            }
        }
        // Top-level `enable_thinking` — Qwen / DashScope.
        "qwen" => {
            body["enable_thinking"] = json!(effort != "none" && effort != "off");
        }
        // `chat_template_kwargs` — local Qwen-compatible servers.
        "qwen-chat-template" => {
            body["chat_template_kwargs"] = json!({
                "enable_thinking": effort != "none" && effort != "off",
                "preserve_thinking": true,
            });
        }
        // Configurable `chat_template_kwargs` (vLLM / HF chat templates).
        "chat-template" => {
            let mut kwargs = compat
                .chat_template_kwargs
                .as_ref()
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            for (_, v) in kwargs.iter_mut() {
                resolve_thinking_var(v, effort, compat);
            }
            // `omitWhenOff` placeholders resolve to Null — drop the key rather
            // than send an explicit null the template may not expect.
            kwargs.retain(|_, v| !v.is_null());
            body["chat_template_kwargs"] = serde_json::Value::Object(kwargs);
        }
        // `chat_template_args` — Baseten.
        "baseten" => {
            let mut args = compat
                .chat_template_args
                .as_ref()
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            for (_, v) in args.iter_mut() {
                resolve_thinking_var(v, effort, compat);
            }
            // `omitWhenOff` placeholders resolve to Null — drop the key rather
            // than send an explicit null the template may not expect.
            args.retain(|_, v| !v.is_null());
            body["chat_template_args"] = serde_json::Value::Object(args);
        }
        // DeepSeek's `thinking: { type: "enabled" | "disabled" }`.
        "deepseek" => {
            let enabled = effort != "none" && effort != "off";
            body["thinking"] = json!({ "type": if enabled { "enabled" } else { "disabled" } });
        }
        // `thinking: { type: … }` — ZAI.
        "zai" => {
            let enabled = effort != "none" && effort != "off";
            body["thinking"] = json!({ "type": if enabled { "enabled" } else { "disabled" } });
        }
        // `thinking: "…"` as a bare string.
        "string-thinking" => {
            body["thinking"] = json!(effort);
        }
        // `ant-ling` sends `thinking: { type: "enabled" }` like DeepSeek.
        "ant-ling" => {
            let enabled = effort != "none" && effort != "off";
            body["thinking"] = json!({ "type": if enabled { "enabled" } else { "disabled" } });
        }
        // `openai` and anything unrecognised.
        _ => {
            body["reasoning_effort"] = json!(effort);
        }
    }
}

/// Substitute a `{ "$var": "thinking.…" }` placeholder with the value of that
/// thinking knob (pi's `chatTemplateKwargs` / `chatTemplateArgs` syntax).
/// Non-placeholder values pass through untouched.
fn resolve_thinking_var(
    v: &mut serde_json::Value,
    effort: &str,
    _compat: &crate::config::ProviderCompat,
) {
    let Some(obj) = v.as_object() else {
        return;
    };
    let Some(var) = obj.get("$var").and_then(|x| x.as_str()) else {
        return;
    };
    let enabled = effort != "none" && effort != "off";
    // `omitWhenOff` drops the key entirely when thinking is disabled — the
    // caller re-reads this sentinel and removes it.
    let omit = obj.get("omitWhenOff").and_then(|x| x.as_bool()).unwrap_or(false);
    if !enabled && omit {
        *v = serde_json::Value::Null;
        return;
    }
    *v = match var {
        "thinking.enabled" => json!(enabled),
        "thinking.effort" => json!(effort),
        // The budget itself is the engine's `thinkingBudgets` concern; the
        // dialect only needs a boolean/enum here.
        "thinking.budget" => json!(effort),
        _ => return,
    };
}

#[async_trait]
impl LlmProvider for GenericOpenAiProvider {
    fn id(&self) -> &'static str {
        "openai_compat"
    }

    fn capabilities(&self) -> crate::provider::Capabilities {
        let mut caps = crate::provider::Capabilities::all();
        // `compat.supports_reasoning_effort` is the per-deployment kill
        // switch for shims that reject the field outright.
        if let Some(allowed) = self
            .compat
            .as_ref()
            .and_then(|c| c.supports_reasoning_effort)
        {
            caps.reasoning = allowed;
        }
        caps
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
        params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        // Model-level `compat` merged over the provider's for this request —
        // `maxTokensField` and `supportsReasoningEffort` are commonly declared
        // per model, so reading the provider's copy alone would silently
        // ignore a model-level declaration.
        let compat = params.compat_with(self.compat.as_ref());
        // `temperature` is optional — some OpenAI-compat backends reject
        // extreme values (verified: devin upstream-errors on temperature=0).
        // Only include it when the caller picked a non-default value; the
        // provider's own default (usually 1.0) applies when omitted.
        let mut body = json!({
            "model": model,
            // `is_error` is replay-only metadata persisted in the session
            // snapshot — strip it from the wire or strict OpenAI-compat
            // backends reject the unknown field. `build_messages` also
            // repairs tool-call pairing — a dangling `tool_calls` entry or
            // orphan tool row is a hard 400 on strict backends (deepseek:
            // "No tool output found for tool call …").
            "messages": build_messages_dev(messages, &compat),
            "stream": true,
        });
        if temperature > 0.0 {
            body["temperature"] = json!(temperature);
        }
        if let Some(store) = compat.supports_store {
            body["store"] = json!(store);
        }
        // Output cap — the model's `maxTokens`, under whichever field name the
        // provider declares (`max_completion_tokens` by default, `max_tokens`
        // for the many shims that never adopted the newer name).
        params.apply_max_tokens(&mut body, compat.max_tokens_field_name());
        if let Some(effort) = reasoning_effort {
            // Capability gate, then the dialect: `openai` sends
            // `reasoning_effort`; other formats express the same intent
            // through their own field (see `apply_thinking`).
            if compat.supports_reasoning_effort.unwrap_or(true) {
                apply_thinking(&mut body, &compat, effort);
            }
        }
        // `stream_options.include_usage` is an OpenAI extension — some
        // OpenAI-compatible backends (devin, certain proxies) reject the
        // field outright instead of ignoring it, so we don't send it. Usage
        // still arrives on the final chunk when the backend provides it.
        // (Verified: nyanya/devin returns `invalid_argument` for it.)
        if let Some(t) = tools {
            body["tools"] = t;
        }
        // `samplingParams` merges last so its keys win over everything above
        // (pi: "its keys win") — the single source of sampling truth.
        params.apply_sampling_params(&mut body);

        let req = self
            .transport
            .post(&self.chat_completions_url(), &body)
            .bearer_auth(&self.api_key);
        let data = self
            .transport
            .send_sse(req, &body, "chat completions request")
            .await?;
        // Track the last-seen usage block so a [DONE] sentinel can fold it
        // into `Done` — and a clean-close ending (no sentinel) still emits it.
        // Shared with the tail fallback: a plain `Copy` capture snapshots
        // at construction, so the synthesized Done would never see usage
        // (and always fire even after a real Done).
        let pending_usage = std::sync::Arc::new(std::sync::Mutex::new(None::<WireUsage>));
        let done = DoneGuard::new();
        let pending2 = pending_usage.clone();
        let flag = done.clone();

        let stream = data.flat_map(move |res| -> futures::stream::Iter<std::vec::IntoIter<anyhow::Result<StreamChunk>>> {
            let items: Vec<anyhow::Result<StreamChunk>> = match res {
                Err(e) => vec![Err(e)],
                Ok(d) => map_data(&d, &mut *pending2.lock().unwrap())
                    .into_iter()
                    .map(|c| {
                        flag.observe(&c);
                        Ok(c)
                    })
                    .collect(),
            };
            futures::stream::iter(items)
        });

        // Guarantee Done-exactly-once: if the backend closed without [DONE]
        // or a trailing usage chunk, synthesize it at stream end — carrying
        // whatever usage a clean-close trailer delivered.
        let stream = done.finish(stream, move || {
            let u = *pending_usage.lock().unwrap();
            StreamChunk::Done {
                prompt_tokens: u.and_then(|u| u.prompt_tokens),
                completion_tokens: u.and_then(|u| u.completion_tokens),
                cached_tokens: u.and_then(|u| u.cached()),
            }
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, Role, ToolCall};

    fn m(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role, content: Some(text.into()), tool_calls: None,
            tool_call_id: None, is_error: None, notice: None, ts: None, reasoning: None,
            images: Vec::new(),
        }
    }

    fn call(id: &str) -> ToolCall {
        ToolCall { id: id.into(), name: "bash".into(), arguments: "{}".into() }
    }

    /// The stored reasoning trace is ours to replay, not the provider's to
    /// read — strict backends reject unknown fields outright, and no
    /// reasoning-model wire accepts a transcript `reasoning` either.
    #[test]
    fn serialize_message_keeps_reasoning_off_the_wire() {
        let msg = m(Role::Assistant, "hello").with_reasoning(Some("chain of thought".into()));
        let v = serialize_message(&msg);
        assert!(v.get("reasoning").is_none(), "reasoning leaked: {v}");
        assert_eq!(v["content"], "hello");
        assert_eq!(msg.reasoning.as_deref(), Some("chain of thought"), "still replayable");
    }

    /// An empty trace is not a thought block — it must not survive on the row.
    #[test]
    fn with_reasoning_drops_blank_traces() {
        assert!(m(Role::Assistant, "x").with_reasoning(Some("  \n ".into())).reasoning.is_none());
    }

    #[test]
    fn build_messages_repairs_dangling_call() {
        // call_2's output never landed (skipped malformed call, interrupted
        // turn, compacted-away row) — a synthesized placeholder must fill it
        // immediately after the assistant row or strict backends (deepseek)
        // reject the whole request: "No tool output found for tool call …".
        let mut a = m(Role::Assistant, "calling");
        a.tool_calls = Some(vec![call("call_1"), call("call_2")]);
        let mut t = m(Role::Tool, "out1");
        t.tool_call_id = Some("call_1".into());
        let msgs = build_messages(&[
            m(Role::System, "s"), m(Role::User, "q"), a, t, m(Role::Assistant, "done"),
        ]);
        let pos_a = msgs.iter().position(|v| v["tool_calls"].is_array()).unwrap();
        assert_eq!(msgs[pos_a + 1]["tool_call_id"].as_str().unwrap(), "call_1");
        assert_eq!(msgs[pos_a + 2]["tool_call_id"].as_str().unwrap(), "call_2");
        assert_eq!(msgs[pos_a + 2]["role"].as_str().unwrap(), "tool");
        // The final assistant message comes after the synthesized output.
        assert_eq!(msgs[pos_a + 3]["role"].as_str().unwrap(), "assistant");
    }

    #[test]
    fn build_messages_drops_orphan_output() {
        let mut t = m(Role::Tool, "orphan");
        t.tool_call_id = Some("never-called".into());
        let msgs = build_messages(&[m(Role::System, "s"), t, m(Role::User, "q")]);
        assert!(msgs.iter().all(|v| v["role"] != "tool"));
    }

    #[test]
    fn build_messages_synthesizes_empty_call_id() {
        let mut a = m(Role::Assistant, "x");
        a.tool_calls = Some(vec![call("")]);
        let mut t = m(Role::Tool, "out");
        t.tool_call_id = Some(String::new()); // blank pairs positionally
        let msgs = build_messages(&[a, t]);
        let id = msgs[0]["tool_calls"][0]["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("call_synth_"));
        assert_eq!(msgs[1]["tool_call_id"].as_str().unwrap(), id);
    }

    #[test]
    fn build_messages_trailing_dangling_call_gets_output() {
        // History ends mid-tools (cancel before dispatch, crash before
        // persist) — synthesize the missing outputs at the tail.
        let mut a = m(Role::Assistant, "calling");
        a.tool_calls = Some(vec![call("call_z")]);
        let msgs = build_messages(&[a]);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1]["role"].as_str().unwrap(), "tool");
        assert_eq!(msgs[1]["tool_call_id"].as_str().unwrap(), "call_z");
    }
}
