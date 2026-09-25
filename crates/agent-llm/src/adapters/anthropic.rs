//! Anthropic adapter — Messages API (`{base}/v1/messages`) over SSE.
//!
//! Wire mapping:
//!
//! | Wire field                                  | → StreamChunk                       |
//! |---------------------------------------------|-------------------------------------|
//! | `content_block_delta.text_delta`            | `ContentDelta`                      |
//! | `content_block_delta.thinking_delta`        | `ReasoningDelta`                    |
//! | `content_block_start(tool_use)`             | `ToolCallDelta{slot,id,name}`       |
//! | `content_block_delta.input_json_delta`      | `ToolCallDelta{args_delta}`         |
//! | `message_delta`/`message_stop` + usage      | `Done{prompt,completion,cached}`    |
//!
//! Request notes: `system` is a top-level field, and tool results fold back as
//! `tool_result` blocks in a `user` message — consecutive `Role::Tool` history
//! coalesces into one, the only shape Anthropic accepts after a `tool_use` turn.
//! `max_tokens` is mandatory; `reasoning_effort` maps to a thinking budget.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use crate::provider::{BoxStream, LlmProvider, ModelParams};
use crate::transport::{DoneGuard, Transport};
use crate::types::{ChatMessage, NoticeKind, Role, StreamChunk};

/// `anthropic-version` — the stable Messages API contract.
const API_VERSION: &str = "2023-06-01";
/// Required field — 32k covers every current Claude output cap; the model
/// stops on its own stop_reason long before hitting it.
const MAX_TOKENS: u32 = 32_768;

/// `reasoning_effort` → `thinking.budget_tokens`. Claude requires the
/// budget be ≥1024 and strictly below `max_tokens`.
fn thinking_budget(effort: &str) -> Option<u32> {
    match effort {
        "low" => Some(2_048),
        "medium" => Some(8_192),
        "high" => Some(16_384),
        "max" => Some(28_672),
        _ => None,
    }
}

pub struct AnthropicProvider {
    transport: Transport,
    base_url: String,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> anyhow::Result<Self> {
        let base = base_url.into().trim_end_matches('/').to_string();
        Ok(Self {
            transport: Transport::new()?,
            base_url: if base.is_empty() {
                "https://api.anthropic.com".to_string()
            } else {
                base
            },
            api_key: api_key.into(),
        })
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.transport = self.transport.with_header(key, value);
        self
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }
}

/// `data:<mime>;base64,<data>` → `(media_type, data)` for Anthropic's
/// base64 image source.
fn split_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
}

/// Build the Anthropic `messages` array + top-level `system` string.
/// `Role::Tool` results coalesce into single user messages (per module doc).
fn build_messages(messages: &[ChatMessage]) -> (String, Vec<serde_json::Value>) {
    let mut system = String::new();
    let mut out: Vec<serde_json::Value> = Vec::new();

    let mut i = 0;
    while i < messages.len() {
        let m = &messages[i];
        match m.role {
            // UI-only compaction/plan card rows — render metadata, never context.
            Role::System if matches!(m.notice, Some(NoticeKind::Compacted) | Some(NoticeKind::Plan)) => {}
            // The compaction memory note is historical context, not a live
            // instruction — emit it as a user message instead of folding it
            // into the top-level `system` prompt. `push_user` merges it into
            // a preceding user row so roles keep alternating.
            Role::System if m.notice == Some(NoticeKind::CompactedMemory) => {
                let text = m.content.clone().unwrap_or_default();
                push_user(&mut out, vec![json!({"type": "text", "text": text})]);
            }
            Role::System => {
                if let Some(t) = &m.content {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(t);
                }
            }
            Role::User => {
                let mut parts = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        parts.push(json!({"type": "text", "text": t}));
                    }
                }
                for img in &m.images {
                    match img.data_url().as_deref().and_then(split_data_url) {
                        Some((mime, data)) => parts.push(json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": mime, "data": data},
                        })),
                        None => parts.push(json!({
                            "type": "text",
                            "text": format!("(image unavailable: {})", img.path.display()),
                        })),
                    }
                }
                push_user(&mut out, parts);
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        parts.push(json!({"type": "text", "text": t}));
                    }
                }
                for tc in m.tool_calls.iter().flatten() {
                    let input = serde_json::from_str::<serde_json::Value>(&tc.arguments)
                        .unwrap_or_else(|_| json!({}));
                    parts.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": input,
                    }));
                }
                // An assistant message that ONLY carried tool calls must
                // still produce a content array — empty content is rejected.
                if parts.is_empty() {
                    parts.push(json!({"type": "text", "text": ""}));
                }
                out.push(json!({"role": "assistant", "content": parts}));
            }
            Role::Tool => {
                // Gather this run of tool results into ONE user message.
                let mut parts = Vec::new();
                while i < messages.len() && matches!(messages[i].role, Role::Tool) {
                    let t = &messages[i];
                    // A tool that produced images (a screenshot) carries them
                    // INSIDE its `tool_result` — this API takes a content
                    // block array where the text-only shape takes a string.
                    // Sticking to the string when there are no images keeps
                    // the common case byte-identical to before.
                    let content = if t.images.is_empty() {
                        json!(t.content.clone().unwrap_or_default())
                    } else {
                        let mut blocks = vec![json!({
                            "type": "text",
                            "text": t.content.clone().unwrap_or_default(),
                        })];
                        for img in &t.images {
                            match img.data_url().as_deref().and_then(split_data_url) {
                                Some((mime, data)) => blocks.push(json!({
                                    "type": "image",
                                    "source": {"type": "base64", "media_type": mime, "data": data},
                                })),
                                None => blocks.push(json!({
                                    "type": "text",
                                    "text": format!("(image unavailable: {})", img.path.display()),
                                })),
                            }
                        }
                        json!(blocks)
                    };
                    parts.push(json!({
                        "type": "tool_result",
                        "tool_use_id": t.tool_call_id.clone().unwrap_or_default(),
                        "content": content,
                        "is_error": t.is_error.unwrap_or(false),
                    }));
                    i += 1;
                }
                push_user(&mut out, parts);
                continue;
            }
        }
        i += 1;
    }
    (system, out)
}

/// Push a `user`-role message, merging into the previous one when the
/// last emitted row is already `user`. The API requires strict
/// user/assistant alternation — a compaction note, a mid-turn steer, or
/// a tool-result run can otherwise leave two consecutive `user` rows,
/// which is a 400. Mixed text/tool_result parts in one user message are
/// legal, so merging is safe.
fn push_user(out: &mut Vec<serde_json::Value>, parts: Vec<serde_json::Value>) {
    if let Some(last) = out.last_mut() {
        if last["role"] == "user" {
            if let Some(arr) = last["content"].as_array_mut() {
                arr.extend(parts);
                return;
            }
        }
    }
    out.push(json!({"role": "user", "content": parts}));
}

/// OpenAI-shaped `[{type:"function",function:{name,description,parameters}}]`
/// → Anthropic `[{name,description,input_schema}]`.
fn map_tools(tools: &serde_json::Value) -> Option<serde_json::Value> {
    let arr = tools.as_array()?;
    let out: Vec<serde_json::Value> = arr
        .iter()
        .filter_map(|t| {
            let f = &t["function"];
            Some(json!({
                "name": f["name"].as_str()?,
                "description": f["description"].as_str().unwrap_or(""),
                "input_schema": f["parameters"].clone(),
            }))
        })
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(json!(out))
    }
}

/// One SSE `data:` payload → normalized chunks. `usage` is threaded via
/// `pending` so the `message_stop` event can emit the full `Done`.
fn map_event(
    event: &serde_json::Value,
    pending: &mut (Option<u32>, Option<u32>, Option<u32>),
) -> Vec<StreamChunk> {
    let ty = event["type"].as_str().unwrap_or("");
    match ty {
        "message_start" => {
            let u = &event["message"]["usage"];
            pending.0 = u["input_tokens"].as_u64().map(|v| v as u32);
            pending.2 = u["cache_read_input_tokens"].as_u64().map(|v| v as u32);
            Vec::new()
        }
        "content_block_start" => {
            let idx = event["index"].as_u64().unwrap_or(0) as usize;
            let cb = &event["content_block"];
            if cb["type"].as_str() == Some("tool_use") {
                vec![StreamChunk::ToolCallDelta {
                    slot: idx,
                    id: cb["id"].as_str().map(String::from),
                    name: cb["name"].as_str().map(String::from),
                    args_delta: String::new(),
                }]
            } else {
                Vec::new()
            }
        }
        "content_block_delta" => {
            let idx = event["index"].as_u64().unwrap_or(0) as usize;
            let d = &event["delta"];
            match d["type"].as_str().unwrap_or("") {
                "text_delta" => d["text"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .map(|t| vec![StreamChunk::ContentDelta(t.to_string())])
                    .unwrap_or_default(),
                "thinking_delta" => d["thinking"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .map(|t| vec![StreamChunk::ReasoningDelta(t.to_string())])
                    .unwrap_or_default(),
                "input_json_delta" => vec![StreamChunk::ToolCallDelta {
                    slot: idx,
                    id: None,
                    name: None,
                    args_delta: d["partial_json"].as_str().unwrap_or("").to_string(),
                }],
                _ => Vec::new(),
            }
        }
        "message_delta" => {
            pending.1 = event["usage"]["output_tokens"].as_u64().map(|v| v as u32);
            Vec::new()
        }
        "message_stop" => vec![StreamChunk::Done {
            prompt_tokens: pending.0,
            completion_tokens: pending.1,
            cached_tokens: pending.2,
        }],
        "error" => vec![StreamChunk::Error(
            event["error"]["message"]
                .as_str()
                .unwrap_or("anthropic stream error")
                .to_string(),
        )],
        _ => Vec::new(), // ping, content_block_stop, …
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn capabilities(&self) -> crate::provider::Capabilities {
        let mut caps = crate::provider::Capabilities::all();
        // Anthropic's extended-thinking mode rejects `temperature` with a
        // 400 — the constraint is declared here so the request builder and
        // any future caller read it from one place.
        caps.temperature_with_reasoning = false;
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
        let (system, wire_messages) = build_messages(messages);
        // The model's `maxTokens` is the real output cap; the adapter's 32k
        // constant is only the fallback for a model that declares none.
        let cap = params.max_tokens.unwrap_or(MAX_TOKENS);
        let mut body = json!({
            "model": model,
            "max_tokens": cap,
            "stream": true,
            "messages": wire_messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        let thinking_on = reasoning_effort.and_then(thinking_budget).is_some();
        if temperature > 0.0 && !(thinking_on && !self.capabilities().temperature_with_reasoning) {
            body["temperature"] = json!(temperature);
        }
        let mut beta: Option<&str> = None;
        if let Some(effort) = reasoning_effort {
            if let Some(budget) = thinking_budget(effort) {
                body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                // Interleaved thinking is still behind the beta header.
                beta = Some("interleaved-thinking-2025-05-14");
            }
        }
        if let Some(t) = tools.as_ref().and_then(map_tools) {
            body["tools"] = t;
        }

        let mut req = self
            .transport
            .post(&self.messages_url(), &body)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION);
        if let Some(b) = beta {
            req = req.header("anthropic-beta", b);
        }
        let data = self
            .transport
            .send_sse(req, &body, "anthropic messages request")
            .await?;
        // Shared stream state — the flat_map mapper and the tail Done
        // fallback must see the SAME usage/done flags (a `move`-copied
        // Option would fork them, losing usage on the fallback path).
        let pending = std::sync::Arc::new(std::sync::Mutex::new(
            (None::<u32>, None::<u32>, None::<u32>),
        ));
        let done = DoneGuard::new();
        let pending2 = pending.clone();
        let flag = done.clone();

        let stream = data.flat_map(move |res| -> futures::stream::Iter<std::vec::IntoIter<anyhow::Result<StreamChunk>>> {
            let items: Vec<anyhow::Result<StreamChunk>> = match res {
                Err(e) => vec![Err(e)],
                Ok(d) => match serde_json::from_str::<serde_json::Value>(&d) {
                    Ok(ev) => map_event(&ev, &mut *pending2.lock().unwrap())
                        .into_iter()
                        .map(|c| {
                            flag.observe(&c);
                            Ok(c)
                        })
                        .collect(),
                    Err(e) => vec![Ok(StreamChunk::Error(format!(
                        "malformed SSE JSON: {e}; payload: {}",
                        &d[..d.len().min(120)]
                    )))],
                },
            };
            futures::stream::iter(items)
        });

        let stream = done.finish(stream, move || {
            let p = *pending.lock().unwrap();
            StreamChunk::Done {
                prompt_tokens: p.0,
                completion_tokens: p.1,
                cached_tokens: p.2,
            }
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_card_row_is_dropped_from_the_wire() {
        // The card row is UI render metadata (a JSON payload) — it must
        // never reach the provider, neither as system text nor as a user
        // turn.
        let (system, out) = build_messages(&[
            ChatMessage::system("kernel"),
            ChatMessage::compaction(120_000, 40_000, 30, false, "summary text"),
            ChatMessage::user("q"),
        ]);
        assert_eq!(system, "kernel");
        assert_eq!(out.len(), 1);
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("before_tokens"), "card JSON leaked to the wire");
    }

    #[test]
    fn compacted_memory_lands_in_user_not_system() {
        // The note is historical context — it must not concatenate into
        // the top-level `system` prompt beside the kernel instructions.
        let (system, out) = build_messages(&[
            ChatMessage::system("kernel"),
            ChatMessage::compacted_memory("prior summary"),
            ChatMessage::user("q"),
        ]);
        assert_eq!(system, "kernel");
        // The note and the following user turn merge into ONE user row —
        // the API requires strict user/assistant alternation.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"].as_str().unwrap(), "user");
        let text = out[0]["content"].as_array().unwrap().iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("prior summary"));
        assert!(text.contains("q"));
    }

    #[test]
    fn consecutive_user_rows_merge() {
        // A mid-turn steer or injected user line after a real user turn
        // would otherwise emit two consecutive `user` rows — a 400.
        let (_s, out) = build_messages(&[
            ChatMessage::system("kernel"),
            ChatMessage::user("a"),
            ChatMessage::user("b"),
            ChatMessage::assistant("r"),
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"].as_str().unwrap(), "user");
        assert_eq!(out[0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(out[1]["role"].as_str().unwrap(), "assistant");
    }

    /// A tool result with no images keeps the plain string `content` — the
    /// shape every existing request already used.
    #[test]
    fn plain_tool_result_stays_a_string() {
        let mut call = ChatMessage::assistant("");
        call.tool_calls = Some(vec![crate::types::ToolCall {
            id: "call_1".into(),
            name: "screenshot".into(),
            arguments: "{}".into(),
        }]);
        let (_s, out) = build_messages(&[
            ChatMessage::system("kernel"),
            call,
            ChatMessage::tool_result("call_1", "no image here"),
        ]);
        let tr = &out[1]["content"][0];
        assert_eq!(tr["type"], "tool_result");
        assert!(tr["content"].is_string(), "content must stay a string: {tr}");
        assert_eq!(tr["content"], "no image here");
    }

    /// A tool result carrying a screenshot switches `content` to a block
    /// array: the text first, then a native `image` block — this API's own
    /// shape for a tool-produced frame, so no synthetic user turn is needed.
    #[test]
    fn tool_result_with_image_uses_content_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        // Minimal but real PNG bytes — `data_url` only reads the file.
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();

        let mut call = ChatMessage::assistant("");
        call.tool_calls = Some(vec![crate::types::ToolCall {
            id: "call_1".into(),
            name: "screenshot".into(),
            arguments: "{}".into(),
        }]);
        let shot = ChatMessage::tool_result("call_1", "shot: 1568x882")
            .tool_result_with_images(
                crate::types::ImageRef::for_path(png.clone()).into_iter().collect(),
            );

        let (_s, out) = build_messages(&[ChatMessage::system("kernel"), call, shot]);
        let tr = &out[1]["content"][0];
        assert_eq!(tr["type"], "tool_result");
        let blocks = tr["content"].as_array().expect("block array");
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "shot: 1568x882");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert!(blocks[1]["source"]["data"].as_str().is_some_and(|d| !d.is_empty()));
    }

    /// A staged file that vanished degrades to a text note instead of
    /// failing the whole request.
    #[test]
    fn missing_tool_image_degrades_to_a_note() {
        let gone = std::path::PathBuf::from("/nonexistent/shot-gone.png");
        let mut call = ChatMessage::assistant("");
        call.tool_calls = Some(vec![crate::types::ToolCall {
            id: "call_1".into(),
            name: "screenshot".into(),
            arguments: "{}".into(),
        }]);
        let shot = ChatMessage::tool_result("call_1", "text")
            .tool_result_with_images(
                crate::types::ImageRef::for_path(gone).into_iter().collect(),
            );
        let (_s, out) = build_messages(&[ChatMessage::system("kernel"), call, shot]);
        let blocks = out[1]["content"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[1]["type"], "text");
        assert!(blocks[1]["text"].as_str().unwrap().contains("image unavailable"));
    }
}
