//! Gemini adapter — `{base}/models/{model}:streamGenerateContent?alt=sse`.
//!
//! Wire mapping:
//!
//! | Wire field                                   | → StreamChunk                    |
//! |----------------------------------------------|----------------------------------|
//! | `parts[{text}]` (no `thought` flag)          | `ContentDelta`                   |
//! | `parts[{thought:true, text}]`                | `ReasoningDelta`                 |
//! | `parts[{functionCall:{name,args}}]`          | `ToolCallDelta{slot,id,name,args_delta}`  |
//! | `usageMetadata` (terminal chunk)             | `Done{prompt,completion,cached}` |
//!
//! Function calls arrive complete per chunk (not JSON shards), so each gets a
//! synthetic slot with `args_delta = stringify(args)` in one shot. System text
//! becomes top-level `systemInstruction`; tool results ride back as
//! `functionResponse` parts in a `user` turn; auth is `x-goog-api-key`.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use crate::provider::{BoxStream, LlmProvider, ModelParams};
use crate::transport::{DoneGuard, Transport};
use crate::types::{ChatMessage, NoticeKind, Role, StreamChunk};

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
    transport: Transport,
    base_url: String,
    api_key: String,
}

impl GeminiProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> anyhow::Result<Self> {
        let base = base_url.into().trim_end_matches('/').to_string();
        Ok(Self {
            transport: Transport::new()?,
            base_url: if base.is_empty() {
                "https://generativelanguage.googleapis.com/v1beta".to_string()
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
    // Invocation id → tool name, built from assistant `tool_calls` rows as
    // we walk. Gemini's `functionResponse` is keyed by NAME (the wire has
    // no call-id concept), so a `tool_call_id` like `call_0` must resolve
    // back to the real name — replaying the id itself makes upstream see
    // a response for a function that was never called.
    let mut call_names: std::collections::HashMap<String, String> = Default::default();

    let mut i = 0;
    while i < messages.len() {
        let m = &messages[i];
        match m.role {
            // UI-only compaction card row — render metadata, never context.
            Role::System if m.notice == Some(NoticeKind::Compacted) => {}
            // The compaction memory note is historical context, not a
            // live instruction — emit it as a user message instead of a
            // system part. `push_user` merges it into a preceding user
            // row so roles keep alternating.
            Role::System if m.notice == Some(NoticeKind::CompactedMemory) => {
                let text = m.content.clone().unwrap_or_default();
                push_user(&mut out, vec![json!({"text": text})]);
            }
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
                push_user(&mut out, parts);
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        parts.push(json!({"text": t}));
                    }
                }
                for tc in m.tool_calls.iter().flatten() {
                    call_names.insert(tc.id.clone(), tc.name.clone());
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
                    let id = t.tool_call_id.clone().unwrap_or_default();
                    // Resolve the canonical invocation id back to the tool
                    // name. Fallback keeps legacy snapshots working — some
                    // stored the bare name in tool_call_id (an id that
                    // never appeared on a call IS the name).
                    let name = call_names.get(&id).cloned().unwrap_or(id);
                    parts.push(json!({
                        "functionResponse": {
                            "name": name,
                            "response": {
                                "result": t.content.clone().unwrap_or_default(),
                            },
                        },
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
/// last emitted row is already `user` — Gemini requires user/model
/// alternation, and a compaction note or a tool-result run following a
/// user turn would otherwise produce two consecutive `user` entries.
fn push_user(out: &mut Vec<serde_json::Value>, parts: Vec<serde_json::Value>) {
    if let Some(last) = out.last_mut() {
        if last["role"] == "user" {
            if let Some(arr) = last["parts"].as_array_mut() {
                arr.extend(parts);
                return;
            }
        }
    }
    out.push(json!({"role": "user", "parts": parts}));
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
        params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let (system, contents) = build_contents(messages);
        let mut gen_cfg = json!({});
        if temperature > 0.0 {
            gen_cfg["temperature"] = json!(temperature);
        }
        // The model's `maxTokens` — Gemini caps output with
        // `generationConfig.maxOutputTokens`.
        params.apply_max_tokens(&mut gen_cfg, "maxOutputTokens");
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

        let req = self
            .transport
            .post(&self.stream_url(model), &body)
            .header("x-goog-api-key", &self.api_key);
        let data = self
            .transport
            .send_sse(req, &body, "gemini streamGenerateContent")
            .await?;
        // Shared stream state — usage arrives on the terminal chunk; the
        // synthetic-Done fallback reads it, so both must see one copy.
        let usage = std::sync::Arc::new(std::sync::Mutex::new(
            None::<serde_json::Value>,
        ));
        let call_index = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let done = DoneGuard::new();
        let usage2 = usage.clone();
        let calls2 = call_index.clone();
        let flag = done.clone();

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
                                            slot: idx,
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
                            out.into_iter()
                                .map(|c| {
                                    flag.observe(&c);
                                    Ok(c)
                                })
                                .collect()
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

        let stream = done.finish(stream, move || {
            let u = usage.lock().unwrap().clone();
            StreamChunk::Done {
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
            }
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::build_contents;
    use crate::types::{ChatMessage, ToolCall};

    #[test]
    fn tool_result_resolves_name_from_call_id() {
        // Gemini's `functionResponse` is name-keyed — the canonical
        // invocation id (`call_0`) must resolve back through the earlier
        // assistant `tool_calls`, not go on the wire as the name. Two
        // same-name calls keep their response order (Gemini's own
        // correlation rule).
        let mut calls = ChatMessage::assistant("calling");
        calls.tool_calls = Some(vec![
            ToolCall { id: "call_0".into(), name: "search".into(), arguments: "{}".into() },
            ToolCall { id: "call_1".into(), name: "search".into(), arguments: "{}".into() },
        ]);
        let r1 = ChatMessage::tool_result("call_0", "first");
        let r2 = ChatMessage::tool_result("call_1", "second");
        let (_sys, contents) = build_contents(&[calls, r1, r2]);
        let responses = &contents[1]["parts"];
        assert_eq!(responses[0]["functionResponse"]["name"], "search");
        assert_eq!(responses[0]["functionResponse"]["response"]["result"], "first");
        assert_eq!(responses[1]["functionResponse"]["name"], "search");
        assert_eq!(responses[1]["functionResponse"]["response"]["result"], "second");
    }

    #[test]
    fn legacy_name_in_tool_call_id_still_resolves() {
        // Old snapshots stored the bare name in `tool_call_id` — an id
        // that never appeared on a call IS the name.
        let r = ChatMessage::tool_result("search", "out");
        let (_sys, contents) = build_contents(&[r]);
        assert_eq!(contents[0]["parts"][0]["functionResponse"]["name"], "search");
    }

    #[test]
    fn compacted_memory_lands_in_user_not_system() {
        // The note is historical context — it must not concatenate into
        // the system parts beside the kernel instructions.
        let (system, contents) = build_contents(&[
            ChatMessage::system("kernel"),
            ChatMessage::compacted_memory("prior summary"),
            ChatMessage::user("q"),
        ]);
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["text"].as_str().unwrap(), "kernel");
        // Note + user turn merge into ONE user entry — Gemini requires
        // user/model alternation.
        let users: Vec<_> = contents.iter().filter(|c| c["role"] == "user").collect();
        assert_eq!(users.len(), 1);
        let texts: Vec<_> = users[0]["parts"].as_array().unwrap().iter()
            .filter_map(|p| p["text"].as_str())
            .collect();
        assert!(texts.iter().any(|t| t.contains("prior summary")));
        assert!(texts.iter().any(|t| t.contains("q")));
    }
}
