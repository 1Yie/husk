//! Gemini adapter — `{base}/models/{model}:streamGenerateContent?alt=sse`.
//!
//! Wire mapping:
//!
//! | Wire field                                   | → StreamChunk                    |
//! |----------------------------------------------|----------------------------------|
//! | `parts[{text}]` (no `thought` flag)          | `ContentDelta`                   |
//! | `parts[{thought:true, text}]`                | `ReasoningDelta`                 |
//! | `parts[{functionCall:{name,args}}]`          | `ToolCallDelta{index,id,name,args_delta}` |
//! | `usageMetadata` (terminal chunk)             | `Done{prompt,completion,cached}` |
//!
//! Function calls arrive COMPLETE per chunk (not JSON shards) — each gets a
//! synthetic index and `args_delta = stringify(args)` in one shot; the
//! kernel's assembler treats it as a finished call.
//!
//! History notes: system text → top-level `systemInstruction`; tool results
//! ride back as `functionResponse` parts inside a `user` turn (consecutive
//! `Role::Tool` messages coalesce). `x-goog-api-key` is the auth header.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use crate::provider::{BoxStream, LlmProvider};
use crate::sse;
use crate::types::{ChatMessage, Role, StreamChunk};

/// `reasoning_effort` → `thinkingConfig.thinkingBudget` — `-1` lets the
/// model pick dynamically.
fn thinking_budget(effort: &str) -> Option<i64> {
    match effort {
        "low" => Some(1_024),
        "medium" => Some(8_192),
        "high" => Some(24_576),
        "max" => Some(-1),
        _ => None,
    }
}

pub struct GeminiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
}

impl GeminiProvider {
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
                "https://generativelanguage.googleapis.com/v1beta".to_string()
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

    fn stream_url(&self, model: &str) -> String {
        format!("{}/models/{model}:streamGenerateContent?alt=sse", self.base_url)
    }
}

/// `data:<mime>;base64,<data>` → `(mime, data)` for `inlineData` parts.
fn split_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
}

/// Build `contents` + `systemInstruction` — tool results coalesce into
/// `functionResponse` parts on one user turn (module doc).
fn build_contents(messages: &[ChatMessage]) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut system: Vec<serde_json::Value> = Vec::new();
    let mut out: Vec<serde_json::Value> = Vec::new();

    let mut i = 0;
    while i < messages.len() {
        let m = &messages[i];
        match m.role {
            Role::System => {
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        system.push(json!({"text": t}));
                    }
                }
            }
            Role::User => {
                let mut parts = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        parts.push(json!({"text": t}));
                    }
                }
                for img in &m.images {
                    match img.data_url().as_deref().and_then(split_data_url) {
                        Some((mime, data)) => parts.push(json!({
                            "inlineData": {"mimeType": mime, "data": data},
                        })),
                        None => parts.push(json!({
                            "text": format!("(image unavailable: {})", img.path.display()),
                        })),
                    }
                }
                out.push(json!({"role": "user", "parts": parts}));
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        parts.push(json!({"text": t}));
                    }
                }
                for tc in m.tool_calls.iter().flatten() {
                    let args = serde_json::from_str::<serde_json::Value>(&tc.arguments)
                        .unwrap_or_else(|_| json!({}));
                    parts.push(json!({
                        "functionCall": {"name": tc.name, "args": args},
                    }));
                }
                if parts.is_empty() {
                    parts.push(json!({"text": ""}));
                }
                out.push(json!({"role": "model", "parts": parts}));
            }
            Role::Tool => {
                let mut parts = Vec::new();
                while i < messages.len() && matches!(messages[i].role, Role::Tool) {
                    let t = &messages[i];
                    // Gemini keys functionResponse by tool NAME (no id
                    // concept) — the call id holds name in OpenAI shape, so
                    // recover the name from our flat ToolCall when present;
                    // tool_call_id here is the call's name already on
                    // replay (tool_result messages store the name in id).
                    parts.push(json!({
                        "functionResponse": {
                            "name": t.tool_call_id.clone().unwrap_or_default(),
                            "response": {
                                "result": t.content.clone().unwrap_or_default(),
                            },
                        },
                    }));
                    i += 1;
                }
                out.push(json!({"role": "user", "parts": parts}));
                continue;
            }
        }
        i += 1;
    }
    (system, out)
}

/// OpenAI `[{type:"function",function:{…}}]` → Gemini
/// `[{functionDeclarations:[{name,description,parameters}]}]`.
fn map_tools(tools: &serde_json::Value) -> Option<serde_json::Value> {
    let arr = tools.as_array()?;
    let decls: Vec<serde_json::Value> = arr
        .iter()
        .filter_map(|t| {
            let f = &t["function"];
            Some(json!({
                "name": f["name"].as_str()?,
                "description": f["description"].as_str().unwrap_or(""),
                "parameters": f["parameters"].clone(),
            }))
        })
        .collect();
    if decls.is_empty() {
        None
    } else {
        Some(json!([{ "functionDeclarations": decls }]))
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn id(&self) -> &'static str {
        "gemini"
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let (system, contents) = build_contents(messages);
        let mut gen_cfg = json!({});
        if temperature > 0.0 {
            gen_cfg["temperature"] = json!(temperature);
        }
        if let Some(effort) = reasoning_effort {
            if let Some(budget) = thinking_budget(effort) {
                gen_cfg["thinkingConfig"] =
                    json!({"thinkingBudget": budget, "includeThoughts": true});
            }
        }
        let mut body = json!({ "contents": contents });
        if !system.is_empty() {
            body["systemInstruction"] = json!({"parts": system});
        }
        if gen_cfg.as_object().is_some_and(|o| !o.is_empty()) {
            body["generationConfig"] = gen_cfg;
        }
        if let Some(t) = tools.as_ref().and_then(map_tools) {
            body["tools"] = t;
        }

        let mut req = self
            .client
            .post(self.stream_url(model))
            .header("x-goog-api-key", &self.api_key)
            .header("Accept", "text/event-stream")
            .json(&body);
        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        if std::env::var("AGENT_DUMP_REQ").is_ok() {
            eprintln!("\n===REQ===\n{}\n===/REQ===", serde_json::to_string(&body).unwrap());
        }

        let resp = req.send().await.context("gemini streamGenerateContent")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "provider returned {status}: {}",
                &text[..text.len().min(512)]
            ));
        }

        let data = sse::data_lines(resp.bytes_stream());
        // Shared stream state — usage arrives on the terminal chunk; the
        // synthetic-Done fallback reads it, so both must see one copy.
        let usage = std::sync::Arc::new(std::sync::Mutex::new(
            None::<serde_json::Value>,
        ));
        let call_index = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let done_emitted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let usage2 = usage.clone();
        let calls2 = call_index.clone();
        let done2 = done_emitted.clone();

        let stream = data.flat_map(move |res| -> futures::stream::Iter<std::vec::IntoIter<anyhow::Result<StreamChunk>>> {
            let items: Vec<anyhow::Result<StreamChunk>> = match res {
                Err(e) => vec![Err(e)],
                Ok(d) => {
                    match serde_json::from_str::<serde_json::Value>(&d) {
                        Ok(ev) => {
                            let mut out = Vec::new();
                            if let Some(u) = ev.get("usageMetadata") {
                                *usage2.lock().unwrap() = Some(u.clone());
                            }
                            for cand in ev["candidates"].as_array().into_iter().flatten() {
                                for part in cand["content"]["parts"].as_array().into_iter().flatten() {
                                    if part.get("functionCall").is_some() {
                                        let fc = &part["functionCall"];
                                        let idx = calls2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        out.push(StreamChunk::ToolCallDelta {
                                            index: idx,
                                            id: Some(format!("call_{idx}")),
                                            name: fc["name"].as_str().map(String::from),
                                            args_delta: serde_json::to_string(
                                                fc.get("args").unwrap_or(&json!({})),
                                            )
                                            .unwrap_or_else(|_| "{}".into()),
                                        });
                                        continue;
                                    }
                                    let text = part["text"].as_str().unwrap_or("");
                                    if text.is_empty() {
                                        continue;
                                    }
                                    if part["thought"].as_bool().unwrap_or(false) {
                                        out.push(StreamChunk::ReasoningDelta(text.to_string()));
                                    } else {
                                        out.push(StreamChunk::ContentDelta(text.to_string()));
                                    }
                                }
                            }
                            out.into_iter().map(Ok).collect()
                        }
                        Err(e) => vec![Ok(StreamChunk::Error(format!(
                            "malformed SSE JSON: {e}; payload: {}",
                            &d[..d.len().min(120)]
                        )))],
                    }
                }
            };
            futures::stream::iter(items)
        });

        let stream = stream.chain(
            futures::stream::once(async move {
                if done_emitted.load(std::sync::atomic::Ordering::Relaxed) {
                    return None;
                }
                let u = usage.lock().unwrap().clone();
                Some(Ok(StreamChunk::Done {
                    prompt_tokens: u
                        .as_ref()
                        .and_then(|v| v["promptTokenCount"].as_u64())
                        .map(|v| v as u32),
                    completion_tokens: u
                        .as_ref()
                        .and_then(|v| v["candidatesTokenCount"].as_u64())
                        .map(|v| v as u32),
                    cached_tokens: u
                        .as_ref()
                        .and_then(|v| v["cachedContentTokenCount"].as_u64())
                        .map(|v| v as u32),
                }))
            })
            .filter_map(|x| async move { x }),
        );

        Ok(Box::pin(stream))
    }
}
