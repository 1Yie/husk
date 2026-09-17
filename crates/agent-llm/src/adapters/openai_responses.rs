//! OpenAI Responses API adapter (`/v1/responses`) — the newer protocol that
//! superseded chat/completions for reasoning models. Devin's
//! `api: "openai-responses"` config maps here.
//!
//! Wire mapping — Responses streams `event:` + `data:` pairs where the
//! `data.type` discriminates the event:
//!
//! | `data.type`                                  | → StreamChunk            |
//! |----------------------------------------------|--------------------------|
//! | `response.reasoning_summary_text.delta`      | `ReasoningDelta`         |
//! | `response.output_text.delta`                 | `ContentDelta`           |
//! | `response.output_item.added` (function_call) | `ToolCallDelta{name,id}` |
//! | `response.function_call_arguments.delta`     | `ToolCallDelta{args}`    |
//! | `response.completed`                         | `Done{usage}`            |
//! | `response.failed` / `response.error`         | `Error`                  |
//!
//! Input shape: `input` is an array of items — `{role, content:[{type:
//! "input_text", text}]}` for user/system, `{type:"function_call_output",
//! call_id, output}` for tool results, and assistant history replays as
//! `{type:"message"/"function_call"}` items. `instructions` carries the
//! system prompt separately.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::provider::{BoxStream, LlmProvider};
use crate::sse;
use crate::types::{ChatMessage, Role, StreamChunk};

/// `/v1/responses` provider — shares the `GenericOpenAiProvider` HTTP/auth
/// shell but speaks the Responses protocol.
pub struct OpenAiResponsesProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
}

impl OpenAiResponsesProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(4)
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            extra_headers: Vec::new(),
        })
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
        self
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url)
    }

    /// `ChatMessage` list → Responses `input` array + `instructions`.
    /// System messages become `instructions`; everything else maps to items.
    ///
    /// Integrity pass (verified against devin upstream): a `function_call_output`
    /// whose `call_id` has no matching `function_call` item earlier in the input
    /// is rejected upstream as `invalid_argument` → `internal_server_error`. We
    /// track emitted call ids and DROP orphan outputs (a session whose assistant
    /// tool_calls row was lost to an old persistence bug). Likewise an assistant
    /// message with empty content AND no tool calls is dropped — it replays as a
    /// blank turn.
    fn build_input(messages: &[ChatMessage]) -> (Vec<Value>, Option<String>) {
        let mut input = Vec::new();
        let mut instructions = Vec::new();
        let mut seen_call_ids: std::collections::HashSet<String> = Default::default();
        for m in messages {
            let text = m.content.clone().unwrap_or_default();
            match m.role {
                Role::System => instructions.push(text),
                Role::User => input.push(json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": text }],
                })),
                Role::Assistant => {
                    // An assistant message with tool_calls replays as
                    // function_call items; a plain one is a message.
                    if let Some(calls) = &m.tool_calls {
                        for c in calls {
                            seen_call_ids.insert(c.id.clone());
                            input.push(json!({
                                "type": "function_call",
                                "call_id": c.id,
                                "name": c.name,
                                "arguments": c.arguments,
                            }));
                        }
                    }
                    if !text.is_empty() {
                        input.push(json!({
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }],
                        }));
                    }
                }
                Role::Tool => {
                    // Tool results are function_call_output items — but ONLY
                    // when a matching function_call was emitted. An orphan
                    // output (call_id never seen) makes devin's upstream
                    // reject the whole request as invalid_argument.
                    let call_id = m.tool_call_id.clone().unwrap_or_default();
                    if seen_call_ids.contains(&call_id) {
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": text,
                        }));
                    }
                    // else: orphan tool output — drop it (and its blank
                    // assistant row is already absent since tool_calls=None).
                }
            }
        }
        let instructions = if instructions.is_empty() {
            None
        } else {
            Some(instructions.join("\n\n"))
        };
        (input, instructions)
    }

    /// Kernel's OpenAI-style `[{type:"function",function:{...}}]` tools →
    /// Responses `[{type:"function",name,description,parameters}]`.
    fn build_tools(tools: Option<Value>) -> Option<Value> {
        let arr = tools?.as_array()?.clone();
        let out: Vec<Value> = arr
            .into_iter()
            .map(|t| {
                if t.get("function").is_some() {
                    let f = &t["function"];
                    json!({
                        "type": "function",
                        "name": f["name"],
                        "description": f.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": f.get("parameters").cloned().unwrap_or(json!({})),
                    })
                } else {
                    t // already Responses-shaped
                }
            })
            .collect();
        Some(Value::Array(out))
    }
}

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    fn id(&self) -> &'static str {
        "openai_responses"
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let (input, instructions) = Self::build_input(messages);
        let mut body = json!({
            "model": model,
            "input": input,
            "stream": true,
        });
        if let Some(ins) = instructions {
            body["instructions"] = json!(ins);
        }
        if temperature > 0.0 {
            body["temperature"] = json!(temperature);
        }
        if let Some(t) = Self::build_tools(tools) {
            body["tools"] = t;
        }

        let mut req = self
            .client
            .post(self.responses_url())
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .json(&body);
        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        if std::env::var("AGENT_DUMP_REQ").is_ok() {
            eprintln!("\n===REQ(responses)===\n{}\n===/REQ===",
                serde_json::to_string(&body).unwrap());
        }

        let resp = req.send().await.context("responses request")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "provider returned {status}: {}",
                &text[..text.len().min(512)]
            ));
        }

        let data = sse::data_lines(resp.bytes_stream());
        let mut usage: Option<(u32, u32)> = None;
        let mut done_emitted = false;
        // devin/swe-2 emits a text-protocol `[call: name(args)]` line inside
        // `output_text.delta` IN ADDITION TO the structured `function_call`
        // item — a duplicate that must not reach history (the model parrots
        // it next turn and the kernel dispatches a bogus empty-name call).
        // The stripper buffers deltas so a `[call:` split across frames is
        // still caught.
        let mut stripper = CallStripper::new();

        let stream = data.flat_map(move |res| {
            let items: Vec<anyhow::Result<StreamChunk>> = match res {
                Err(e) => vec![Err(e)],
                Ok(d) => {
                    let evs = map_data(&d, &mut usage);
                    evs.into_iter()
                        .filter_map(|c| match c {
                            StreamChunk::ContentDelta(t) => {
                                stripper.feed(&t).map(StreamChunk::ContentDelta)
                            }
                            // A real tool-call item flushes any held text
                            // (a partial `[call:` echo is dropped — the
                            // structured call carries the intent).
                            StreamChunk::ToolCallDelta { .. } => {
                                stripper.flush_call_line();
                                Some(c)
                            }
                            StreamChunk::Done { .. } => {
                                done_emitted = true;
                                Some(c)
                            }
                            other => Some(other),
                        })
                        .map(Ok)
                        .collect()
                }
            };
            futures::stream::iter(items)
        });

        let stream = stream.chain(futures::stream::once(async move {
            if done_emitted {
                None
            } else {
                Some(Ok(StreamChunk::Done {
                    prompt_tokens: usage.map(|u| u.0),
                    completion_tokens: usage.map(|u| u.1),
                }))
            }
        }).filter_map(|x| async move { x }));

        Ok(Box::pin(stream))
    }
}

// ---- wire types -----------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RespEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    item: Option<RespItem>,
    /// `output_index` groups output items (reasoning=0, first call=1, …) —
    /// the assembler keys on it, NOT a hardcoded 0.
    #[serde(default)]
    output_index: Option<usize>,
    /// `item_id` on argument deltas references the call's item id.
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    response: Option<RespCompleted>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RespItem {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RespCompleted {
    #[serde(default)]
    usage: Option<RespUsage>,
}

#[derive(Debug, Deserialize)]
struct RespUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

/// Incremental `[call: …]` stripper — buffers text deltas and removes any
/// `[call: name(args)]` / `[call: ()]` fragments (a text-protocol tool-call
/// echo some backends emit alongside the real structured call). Handles a
/// `[call:` token split across delta boundaries by holding the tail.
struct CallStripper {
    /// Text held back because it might be a partial `[call:` prefix or
    /// inside an unterminated `[call: …` line.
    hold: String,
    /// True once we've entered a `[call:` line (until `]` or newline).
    in_call: bool,
}

impl CallStripper {
    fn new() -> Self {
        Self { hold: String::new(), in_call: false }
    }

    /// Feed a delta; returns the safe-to-emit text (None = fully held).
    /// A `[call: …]` line is dropped WITH its surrounding newlines so the
    /// stripped text doesn't leave blank-line artifacts.
    fn feed(&mut self, delta: &str) -> Option<String> {
        self.hold.push_str(delta);
        let mut out = String::new();
        let mut chars = self.hold.chars().peekable();
        let mut consumed = 0usize;
        while let Some(c) = chars.next() {
            if self.in_call {
                consumed += c.len_utf8();
                if c == ']' || c == '\n' {
                    self.in_call = false;
                    // Swallow a trailing newline right after the call line.
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                        consumed += 1;
                    }
                }
                continue;
            }
            if c == '[' {
                let rest: String = chars.clone().take(5).collect();
                if rest.starts_with("call:") {
                    self.in_call = true;
                    consumed += c.len_utf8();
                    // A `[call:` line usually follows a newline — drop the
                    // newline we already emitted so no blank line remains.
                    if out.ends_with('\n') {
                        out.pop();
                    }
                    continue;
                }
                // Partial `[call` prefix possibly split across deltas — hold.
                if "call:".starts_with(&rest) && !rest.is_empty() {
                    self.hold = self.hold[consumed..].to_string();
                    return if out.is_empty() { None } else { Some(out) };
                }
            }
            out.push(c);
            consumed += c.len_utf8();
        }
        self.hold = self.hold[consumed..].to_string();
        if out.is_empty() { None } else { Some(out) }
    }

    /// Flush any held non-call text (on a real tool call or stream end).
    /// Returns a marker so the caller can still emit the call chunk.
    fn flush_call_line(&mut self) -> Option<()> {
        // Any held text that wasn't a `[call:` is safe to emit — but at a
        // tool-call boundary we just clear the buffer (the structured call
        // carries the intent; a partial text echo is dropped).
        self.hold.clear();
        self.in_call = false;
        Some(())
    }
}

/// One `data:` payload → zero-or-more `StreamChunk`s.
fn map_data(data: &str, usage: &mut Option<(u32, u32)>) -> Vec<StreamChunk> {
    if data.trim() == "[DONE]" {
        return vec![];
    }
    let ev: RespEvent = match serde_json::from_str(data) {
        Ok(e) => e,
        Err(_) => return vec![], // non-JSON frame — ignore
    };
    let mut out = Vec::new();
    match ev.kind.as_str() {
        "response.reasoning_summary_text.delta" => {
            if let Some(d) = ev.delta {
                if !d.is_empty() {
                    out.push(StreamChunk::ReasoningDelta(d));
                }
            }
        }
        "response.output_text.delta" => {
            if let Some(d) = ev.delta {
                if !d.is_empty() {
                    out.push(StreamChunk::ContentDelta(d));
                }
            }
        }
        "response.output_item.added" => {
            if let Some(item) = ev.item {
                if item.kind == "function_call" {
                    // call_id like `list_dir:0#hash` — the *name* is the
                    // tool, call_id is the correlation id.
                    out.push(StreamChunk::ToolCallDelta {
                        index: ev.output_index.unwrap_or(0),
                        id: item.call_id,
                        name: item.name,
                        args_delta: String::new(),
                    });
                }
            }
        }
        "response.function_call_arguments.delta" => {
            if let Some(d) = ev.delta {
                // item_id correlates the args shard to its call — send it as
                // the id so the assembler merges by output_index slot.
                out.push(StreamChunk::ToolCallDelta {
                    index: ev.output_index.unwrap_or(0),
                    id: ev.item_id.clone().or(ev.call_id),
                    name: ev.name,
                    args_delta: d,
                });
            }
        }
        "response.completed" => {
            if let Some(r) = ev.response {
                if let Some(u) = r.usage {
                    *usage = Some((
                        u.input_tokens.unwrap_or(0),
                        u.output_tokens.unwrap_or(0),
                    ));
                }
            }
            out.push(StreamChunk::Done {
                prompt_tokens: usage.map(|u| u.0),
                completion_tokens: usage.map(|u| u.1),
            });
        }
        "response.failed" | "response.error" | "error" => {
            let msg = ev
                .error
                .map(|e| e.to_string())
                .unwrap_or_else(|| format!("responses {}", ev.kind));
            out.push(StreamChunk::Error(msg));
        }
        _ => {} // output_item.done, content_part.*, created — display-irrelevant
    }
    out
}

#[cfg(test)]
mod tests {
    use super::CallStripper;

    #[test]
    fn strips_call_lines() {
        let mut s = CallStripper::new();
        // A `[call: ()]` + `[call: bash({...})]` echo mixed into text.
        let out: Vec<String> = [
            "已创建 ", "\n[call: ()]\n", "[call: bash({\"command\": \"x\"})]\n", "done.",
        ]
        .iter()
        .filter_map(|d| s.feed(d))
        .collect();
        assert_eq!(out.concat(), "已创建 done.");
    }

    #[test]
    fn strips_call_split_across_deltas() {
        let mut s = CallStripper::new();
        // `[call:` split mid-token across two deltas.
        let a = s.feed("hello [ca");
        let b = s.feed("ll: foo}]\nworld");
        assert_eq!(a.unwrap_or_default(), "hello ");
        assert_eq!(b.unwrap_or_default(), "world");
    }

    #[test]
    fn keeps_bracket_text_that_isnt_call() {
        let mut s = CallStripper::new();
        let out = s.feed("see [docs] and [more] here");
        assert_eq!(out.as_deref(), Some("see [docs] and [more] here"));
    }
}
