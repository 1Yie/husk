//! Anthropic adapter — Messages API (`{base}/v1/messages`) over SSE.
//!
//! Wire mapping:
//!
//! | Wire field                                  | → StreamChunk                       |
//! |---------------------------------------------|-------------------------------------|
//! | `content_block_delta.text_delta`            | `ContentDelta`                      |
//! | `content_block_delta.thinking_delta`        | `ReasoningDelta`                    |
//! | `content_block_start(tool_use)`             | `ToolCallDelta{index,id,name}`      |
//! | `content_block_delta.input_json_delta`      | `ToolCallDelta{args_delta}`         |
//! | `message_delta`/`message_stop` + usage      | `Done{prompt,completion,cached}`    |
//!
//! Request notes: `system` is a top-level field (never a message); tool
//! results fold back in as `tool_result` blocks inside a `user` message —
//! consecutive `Role::Tool` history coalesces into ONE user message, the
//! only shape Anthropic accepts after a `tool_use` turn. `max_tokens` is
//! mandatory; `thinking` is enabled via `reasoning_effort` → budget map.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use crate::provider::{BoxStream, LlmProvider};
use crate::sse;
use crate::types::{ChatMessage, Role, StreamChunk};

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
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
}

impl AnthropicProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(4)
            .build()
            .context("build reqwest client")?;
        let base = base_url.into().trim_end_matches('/').to_string();
        Ok(Self {
            client,
            base_url: if base.is_empty() {
                "https://api.anthropic.com".to_string()
            } else {
                base
            },
            api_key: api_key.into(),
            extra_headers: Vec::new(),
        })
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
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
                out.push(json!({"role": "user", "content": parts}));
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
                    parts.push(json!({
                        "type": "tool_result",
                        "tool_use_id": t.tool_call_id.clone().unwrap_or_default(),
                        "content": t.content.clone().unwrap_or_default(),
                        "is_error": t.is_error.unwrap_or(false),
                    }));
                    i += 1;
                }
                out.push(json!({"role": "user", "content": parts}));
                continue;
            }
        }
        i += 1;
    }
    (system, out)
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
                    index: idx,
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
                    index: idx,
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

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let (system, wire_messages) = build_messages(messages);
        let mut body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "messages": wire_messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if temperature > 0.0 {
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
            .client
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("Accept", "text/event-stream")
            .json(&body);
        if let Some(b) = beta {
            req = req.header("anthropic-beta", b);
        }
        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        if std::env::var("AGENT_DUMP_REQ").is_ok() {
            eprintln!("\n===REQ===\n{}\n===/REQ===", serde_json::to_string(&body).unwrap());
        }

        let resp = req.send().await.context("anthropic messages request")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "provider returned {status}: {}",
                &text[..text.len().min(512)]
            ));
        }

        let data = sse::data_lines(resp.bytes_stream());
        // Shared stream state — the flat_map mapper and the tail Done
        // fallback must see the SAME usage/done flags (a `move`-copied
        // Option would fork them, losing usage on the fallback path).
        let pending = std::sync::Arc::new(std::sync::Mutex::new(
            (None::<u32>, None::<u32>, None::<u32>),
        ));
        let done_emitted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pending2 = pending.clone();
        let done2 = done_emitted.clone();

        let stream = data.flat_map(move |res| -> futures::stream::Iter<std::vec::IntoIter<anyhow::Result<StreamChunk>>> {
            let items: Vec<anyhow::Result<StreamChunk>> = match res {
                Err(e) => vec![Err(e)],
                Ok(d) => match serde_json::from_str::<serde_json::Value>(&d) {
                    Ok(ev) => map_event(&ev, &mut pending2.lock().unwrap())
                        .into_iter()
                        .map(|c| {
                            if matches!(c, StreamChunk::Done { .. }) {
                                done2.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
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

        let stream = stream.chain(
            futures::stream::once(async move {
                if done_emitted.load(std::sync::atomic::Ordering::Relaxed) {
                    None
                } else {
                    let p = *pending.lock().unwrap();
                    Some(Ok(StreamChunk::Done {
                        prompt_tokens: p.0,
                        completion_tokens: p.1,
                        cached_tokens: p.2,
                    }))
                }
            })
            .filter_map(|x| async move { x }),
        );

        Ok(Box::pin(stream))
    }
}
