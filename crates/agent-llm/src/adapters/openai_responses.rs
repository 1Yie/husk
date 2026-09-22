//! OpenAI Responses API adapter (`/v1/responses`) — the protocol reasoning models
//! use; Devin's `api: "openai-responses"` config maps here.
//!
//! Streams `event:` + `data:` pairs, discriminated by `data.type`:
//!
//! | `data.type`                                  | → StreamChunk            |
//! |----------------------------------------------|--------------------------|
//! | `response.reasoning_summary_text.delta`      | `ReasoningDelta`         |
//! | `response.reasoning_text.delta` (raw CoT)    | `ReasoningDelta`         |
//! | `response.output_text.delta`                 | `ContentDelta`           |
//! | `response.output_item.added` (function_call) | `ToolCallDelta{name,id}` |
//! | `response.function_call_arguments.delta`     | `ToolCallDelta{args}`    |
//! | `response.completed`                         | `Done{usage}`            |
//! | `response.failed` / `response.error`         | `Error`                  |
//!
//! `input` is an array of items — `{role, content:[{type:"input_text", text}]}` for
//! user/system, `{type:"function_call_output", call_id, output}` for tool results,
//! assistant history replayed as `{type:"message"/"function_call"}`. The system
//! prompt rides separately in `instructions`.

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::provider::{BoxStream, LlmProvider, ModelParams};
use crate::transport::{DoneGuard, Transport};
use crate::types::{ChatMessage, Role, StreamChunk};

/// `/v1/responses` provider — shares the `GenericOpenAiProvider` HTTP/auth
/// shell but speaks the Responses protocol.
pub struct OpenAiResponsesProvider {
    transport: Transport,
    base_url: String,
    api_key: String,
    compat: Option<crate::config::ProviderCompat>,
}

impl OpenAiResponsesProvider {
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

    fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url)
    }

    /// `ChatMessage` list → Responses `input` array + `instructions`.
    /// System messages become `instructions`; everything else maps to items.
    ///
    /// Integrity pass (upstream rejects unpaired items as
    /// `invalid_argument`/`400 "No tool output found"`): orphan
    /// `function_call_output` rows (no matching `function_call` earlier)
    /// are dropped, blank assistant rows (empty content AND no tool calls)
    /// are dropped, and a `function_call` whose output never landed gets a
    /// synthesized placeholder emitted right after the call, before the
    /// next non-output item.
    fn build_input(messages: &[ChatMessage]) -> (Vec<Value>, Option<String>) {
        let mut input = Vec::new();
        let mut instructions = Vec::new();
        let mut seen_call_ids: std::collections::HashSet<String> = Default::default();
        // Call ids still awaiting their `function_call_output`, in call
        // order — drained by matching tool rows or by placeholder
        // synthesis at the next non-output boundary.
        let mut expected: Vec<String> = Vec::new();
        // Calls persisted with `id: ""` (a provider that never named them —
        // deepseek's Responses shim before the item.done fix) get synthesized
        // ids here, queued so a tool row with `tool_call_id: ""` still pairs
        // positionally with the Nth id-less call. Two passes would diverge;
        // one queue keeps replay self-consistent for poisoned snapshots.
        let mut blank_ids: std::collections::VecDeque<String> = Default::default();
        let mut synth_seq = 0usize;

        // Missing outputs must sit directly after their call — flush them
        // before any item that isn't a `function_call_output`.
        macro_rules! flush_missing {
            () => {
                for id in expected.drain(..) {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": id,
                        "output": "(tool result missing — the call was interrupted or dropped from history)",
                    }));
                }
            };
        }

        for m in messages {
            let text = m.content.clone().unwrap_or_default();
            match m.role {
                // System rows fold into `instructions` — they never occupy
                // an `input` position, so they must NOT flush `expected`
                // (a mid-block notice would otherwise synthesize outputs
                // prematurely and orphan the real rows after it).
                Role::System => instructions.push(text),
                Role::User => {
                    flush_missing!();
                    // Vision: image refs become `input_image` parts after
                    // the text part — materialized `data:` URLs, degrading
                    // to a text note when the staged file is gone.
                    let mut parts = vec![json!({ "type": "input_text", "text": text })];
                    for img in &m.images {
                        match img.data_url() {
                            Some(url) => parts.push(json!({
                                "type": "input_image",
                                "image_url": url,
                            })),
                            None => parts.push(json!({
                                "type": "input_text",
                                "text": format!("(image unavailable: {})", img.path.display()),
                            })),
                        }
                    }
                    input.push(json!({
                        "role": "user",
                        "content": parts,
                    }));
                }
                Role::Assistant => {
                    flush_missing!();
                    // Text first, then function_call items — the natural
                    // output order ("I'll check the files" → calls), and it
                    // keeps each call adjacent to its outputs instead of
                    // wedging the assistant text between them (strict
                    // shims pair by position).
                    if !text.is_empty() {
                        input.push(json!({
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }],
                        }));
                    }
                    if let Some(calls) = &m.tool_calls {
                        for c in calls {
                            // A `call_id: ""` is a hard 400 upstream —
                            // synthesize a stable id for this pass.
                            let id = if c.id.is_empty() {
                                synth_seq += 1;
                                format!("call_synth_{synth_seq}")
                            } else {
                                c.id.clone()
                            };
                            blank_ids.push_back(id.clone());
                            seen_call_ids.insert(id.clone());
                            expected.push(id.clone());
                            input.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": c.name,
                                "arguments": c.arguments,
                            }));
                        }
                    }
                }
                Role::Tool => {
                    // Tool results are function_call_output items — but ONLY
                    // when a matching function_call was emitted. An orphan
                    // output (call_id never seen) makes devin's upstream
                    // reject the whole request as invalid_argument.
                    let mut call_id = m.tool_call_id.clone().unwrap_or_default();
                    // `tool_call_id: ""` pairs with the earliest unpaired
                    // call — preserves output ordering from the poisoned
                    // snapshot (engine dispatched calls in tool_calls order).
                    if call_id.is_empty() {
                        if let Some(id) = blank_ids.pop_front() {
                            call_id = id;
                        } else if let Some(id) = expected.first() {
                            // No blank-id call outstanding — the blank output
                            // still belongs to the earliest unanswered call
                            // (dispatched in order); pairing it keeps the real
                            // output instead of dropping + synthesizing.
                            call_id = id.clone();
                        }
                    }
                    if seen_call_ids.contains(&call_id) {
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": text,
                        }));
                        if let Some(pos) = expected.iter().position(|id| *id == call_id) {
                            expected.remove(pos);
                        }
                        // A paired call also leaves the blank-id queue —
                        // otherwise a later `tool_call_id: ""` row pops an
                        // already-answered id and emits a SECOND output for
                        // it while the real unanswered call gets a
                        // placeholder (mixed snapshots: some calls kept real
                        // ids, some were persisted blank).
                        if let Some(pos) = blank_ids.iter().position(|id| *id == call_id) {
                            blank_ids.remove(pos);
                        }
                    }
                    // else: orphan tool output — drop it (and its blank
                    // assistant row is already absent since tool_calls=None).
                }
            }
        }
        // Trailing dangling calls — history ends mid-tools (cancel before
        // dispatch fill, crash before persist).
        flush_missing!();
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

    fn capabilities(&self) -> crate::provider::Capabilities {
        let mut caps = crate::provider::Capabilities::all();
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
        // Model-level `compat` merged over the provider's for this request.
        let compat = params.compat_with(self.compat.as_ref());
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
        if let Some(store) = compat.supports_store {
            body["store"] = json!(store);
        }
        // The Responses API caps output with `max_output_tokens` — the model's
        // `maxTokens` under its native field name.
        params.apply_max_tokens(&mut body, "max_output_tokens");
        if let Some(effort) = reasoning_effort {
            if compat.supports_reasoning_effort.unwrap_or(true) {
                body["reasoning"] = json!({ "effort": effort });
            }
        }
        if let Some(t) = Self::build_tools(tools) {
            body["tools"] = t;
        }
        // `samplingParams` merges last so its keys win.
        params.apply_sampling_params(&mut body);

        let req = self
            .transport
            .post(&self.responses_url(), &body)
            .bearer_auth(&self.api_key);
        let data = self
            .transport
            .send_sse(req, &body, "responses request")
            .await?;
        // Shared with the tail fallback — a `Copy` capture would snapshot
        // `None` at construction, so the synthesized Done must read usage
        // through the Arc, not a moved Option.
        let usage = std::sync::Arc::new(std::sync::Mutex::new(
            None::<(u32, u32, u32)>,
        ));
        let done = DoneGuard::new();
        let usage2 = usage.clone();
        let flag = done.clone();
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
                    let evs = map_data(&d, &mut *usage2.lock().unwrap());
                    let mut items: Vec<anyhow::Result<StreamChunk>> = evs
                        .into_iter()
                        .filter_map(|c| match c {
                            StreamChunk::ContentDelta(t) => {
                                stripper.feed(&t).map(StreamChunk::ContentDelta)
                            }
                            // A real tool-call item flushes any held text
                            // (a partial `[call:` echo is dropped — the
                            // structured call carries the intent).
                            StreamChunk::ToolCallDelta { .. } => {
                                stripper.flush_call_line();
                                flag.observe(&c);
                                Some(c)
                            }
                            other => {
                                flag.observe(&other);
                                Some(other)
                            }
                        })
                        .map(Ok)
                        .collect();
                    // `[call: name({json})]` text lines recovered by the
                    // stripper become real ToolCallDelta so swe-2's
                    // text-protocol calls actually execute (not just hidden).
                    // Each recovered text-call gets its own assembler slot —
                    // grows per call so two `[call:]` lines don't
                    // concat into one corrupted slot.
                    let mut slot = 10_000usize;
                    for call in stripper.take_recovered() {
                        items.push(Ok(StreamChunk::ToolCallDelta {
                            slot,
                            id: Some(call.id),
                            name: Some(call.name),
                            args_delta: call.arguments,
                        }));
                        slot += 1;
                    }
                    items
                }
            };
            futures::stream::iter(items)
        });

        let stream = done.finish(stream, move || {
            let u = *usage.lock().unwrap();
            StreamChunk::Done {
                prompt_tokens: u.map(|u| u.0),
                completion_tokens: u.map(|u| u.1),
                cached_tokens: u.map(|u| u.2),
            }
        });

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
    /// The provider's item id (`fc_…`) — fallback correlation key when a
    /// backend omits `call_id` (deepseek fills it only on `item.done`, or
    /// never). Distinct from `call_id` (`call_…`) on the OpenAI wire.
    #[serde(default)]
    id: Option<String>,
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
    /// Responses API: `input_tokens_details.cached_tokens`.
    #[serde(default)]
    input_tokens_details: Option<InputTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct InputTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
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
    /// The accumulating `[call: …` payload — parsed into a ToolCall on `]`.
    call_buf: String,
    /// Tool calls recovered from `[call: name({json})]` text lines — the
    /// swe-2 text-protocol escape hatch. Drained by the adapter.
    recovered: Vec<crate::types::ToolCall>,
}

impl CallStripper {
    fn new() -> Self {
        Self { hold: String::new(), in_call: false, call_buf: String::new(), recovered: Vec::new() }
    }

    /// Feed a delta; returns the safe-to-emit text (None = fully held).
    /// Byte-index scan over `hold` with two hold-back rules:
    ///   * `[` near the end that could still become `[call:` → hold it (a
    ///     `[` split across deltas used to leak the `call:` tail as text).
    ///   * inside a call, `]` only terminates when followed by `\n` or held
    ///     at buffer end — `]` inside JSON args (`result[j]`) must NOT end it.
    fn feed(&mut self, delta: &str) -> Option<String> {
        self.hold.push_str(delta);
        let mut out = String::new();
        let mut i = 0usize; // byte cursor into self.hold
        let bytes = self.hold.as_bytes();
        while i < bytes.len() {
            if self.in_call {
                match bytes[i] {
                    b']' => match bytes.get(i + 1) {
                        Some(b'\n') | Some(b'\r') => {
                            // `]` + newline → call line ends. Parse + drop.
                            if let Some(mut c) = parse_call_line(&self.call_buf) {
                                c.id = format!("textcall-{}-{}", c.name, self.recovered.len());
                                self.recovered.push(c);
                            }
                            self.call_buf.clear();
                            self.in_call = false;
                            i += 2; // skip `]` and the newline
                            continue;
                        }
                        None => break, // `]` at buffer end — wait for next byte
                        _ => {
                            // `]` inside args (JSON index/array) — keep going.
                            self.call_buf.push(']');
                            i += 1;
                            continue;
                        }
                    },
                    _ => {
                        let l = utf8_len(bytes[i]);
                        self.call_buf.push_str(&self.hold[i..i + l]);
                        i += l;
                        continue;
                    }
                }
            }
            if bytes[i] == b'[' {
                let rest = &self.hold[i..];
                if rest.starts_with("[call:") {
                    self.in_call = true;
                    self.call_buf.clear();
                    i += "[call:".len();
                    // The echo usually follows a newline we already emitted —
                    // pop it so no blank line remains.
                    if out.ends_with('\n') {
                        out.pop();
                    }
                    continue;
                }
                if "[call:".starts_with(rest) {
                    break; // `[` / `[ca` / `[call` at buffer end — hold it
                }
                out.push('[');
                i += 1;
                continue;
            }
            let l = utf8_len(bytes[i]);
            out.push_str(&self.hold[i..i + l]);
            i += l;
        }
        self.hold = self.hold[i..].to_string();
        if out.is_empty() { None } else { Some(out) }
    }

    /// Flush on a structured tool-call item or stream end — drops held text,
    /// and if we were mid-call, tries to parse whatever accumulated.
    fn flush_call_line(&mut self) -> Option<()> {
        if self.in_call && !self.call_buf.is_empty() {
            if let Some(mut c) = parse_call_line(&self.call_buf) {
                c.id = format!("textcall-{}-{}", c.name, self.recovered.len());
                self.recovered.push(c);
            }
        }
        self.hold.clear();
        self.call_buf.clear();
        self.in_call = false;
        Some(())
    }

    /// Drain any `[call: name({json})]` lines recovered into real ToolCalls.
    fn take_recovered(&mut self) -> Vec<crate::types::ToolCall> {
        std::mem::take(&mut self.recovered)
    }
}

/// UTF-8 length of the char starting at byte `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 { 1 } else if b < 0xE0 { 2 } else if b < 0xF0 { 3 } else { 4 }
}

/// Parse a `[call: name({json})]` body (the text inside the brackets, after
/// `call:`) into a `ToolCall` — `smart_read({"path":"x"})` →
/// `{id:"textcall-N", name:"smart_read", arguments:{…}}`. `[call: ()]` is
/// a no-op (returns None). Returns None for a non-call or malformed body.
fn parse_call_line(body: &str) -> Option<crate::types::ToolCall> {
    let body = body.trim();
    if body.is_empty() || body == "()" {
        return None;
    }
    // `name(args)` — split at the first `(`.
    let open = body.find('(')?;
    let name = body[..open].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let mut args = body[open..].trim().to_string();
    // Strip the outer parens → just the JSON object.
    if args.starts_with('(') && args.ends_with(')') {
        args = args[1..args.len() - 1].trim().to_string();
    }
    if args.is_empty() || args == "()" {
        args = "{}".into();
    }
    Some(crate::types::ToolCall {
        id: format!("textcall-{}", name),
        name,
        arguments: args,
    })
}

/// One `data:` payload → zero-or-more `StreamChunk`s.
fn map_data(data: &str, usage: &mut Option<(u32, u32, u32)>) -> Vec<StreamChunk> {
    if data.trim() == "[DONE]" {
        return vec![];
    }
    let ev: RespEvent = match serde_json::from_str(data) {
        Ok(e) => e,
        Err(_) => return vec![], // non-JSON frame — ignore
    };
    let mut out = Vec::new();
    match ev.kind.as_str() {
        // Reasoning streams under two event kinds depending on the backend:
        // `reasoning_summary_text.delta` carries OpenAI's *summarized*
        // reasoning, while open-weight models (deepseek's shim) stream the
        // raw CoT as `reasoning_text.delta`. Both fold into ReasoningDelta —
        // without the second arm a thinking model renders as silent.
        "response.reasoning_summary_text.delta"
        | "response.reasoning_text.delta" => {
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
        "response.output_item.added" | "response.output_item.done" => {
            if let Some(item) = ev.item {
                if item.kind == "function_call" {
                    // call_id like `list_dir:0#hash` — the *name* is the
                    // tool, call_id is the correlation id. Some providers
                    // (devin/swe-2) only send the name inside call_id, so
                    // fall back to extracting it when `item.name` is absent
                    // — otherwise the assembler finishes a name="" call and
                    // the engine reports "malformed empty tool call".
                    // `item.done` is handled too: some shims leave `call_id`
                    // empty until the completed item — without this arm the
                    // assembled call keeps id="" and the next replay 400s.
                    let id = item.call_id.or_else(|| ev.call_id.clone()).or(item.id);
                    let name = item.name.or_else(|| {
                        id.as_deref()
                            .and_then(|cid| {
                                // `name:0#hash` → `name`. A plain `call_…`
                                // id has no `:` and yields nothing, so we
                                // don't invent a bogus name from it.
                                cid.split(':').next().filter(|_| cid.contains(':'))
                            })
                            .map(|s| s.to_string())
                            .filter(|s| !s.is_empty())
                    });
                    out.push(StreamChunk::ToolCallDelta {
                        slot: ev.output_index.unwrap_or(0),
                        id,
                        name,
                        args_delta: String::new(),
                    });
                }
            }
        }
        "response.function_call_arguments.delta" => {
            if let Some(d) = ev.delta {
                // item_id correlates the args shard to its call — send it as
                // the id so the assembler merges by output_index slot.
                // Same name fallback: providers that only name the call via
                // `name:0#hash` item_id/call_id get it extracted here too.
                let id = ev.item_id.clone().or(ev.call_id);
                let name = ev.name.or_else(|| {
                    id.as_deref()
                        .and_then(|cid| cid.split(':').next().filter(|_| cid.contains(':')))
                        .map(|s| s.to_string())
                        .filter(|s| !s.is_empty())
                });
                out.push(StreamChunk::ToolCallDelta {
                    slot: ev.output_index.unwrap_or(0),
                    id,
                    name,
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
                        u.input_tokens_details.and_then(|d| d.cached_tokens).unwrap_or(0),
                    ));
                }
            }
            out.push(StreamChunk::Done {
                prompt_tokens: usage.map(|u| u.0),
                completion_tokens: usage.map(|u| u.1),
                cached_tokens: usage.map(|u| u.2),
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
    use super::{map_data, CallStripper, OpenAiResponsesProvider};
    use crate::types::{ChatMessage, StreamChunk, ToolCall};

    #[test]
    fn reasoning_text_delta_maps_to_reasoning_chunk() {
        let mut usage = None;
        let chunks = map_data(
            r#"{"type":"response.reasoning_text.delta","delta":"thinking hard"}"#,
            &mut usage,
        );
        assert!(matches!(
            chunks.first(),
            Some(StreamChunk::ReasoningDelta(t)) if t == "thinking hard"
        ));
    }

    #[test]
    fn build_input_synthesizes_empty_call_ids() {
        // Deepseek's Responses shim never emits call_id — the assembler
        // used to persist `id: ""`, and the next request 400'd on
        // `call_id: empty string`. build_input must synthesize ids and
        // pair blank tool_call_id outputs positionally.
        let mut calls_msg = ChatMessage::assistant("");
        calls_msg.tool_calls = Some(vec![ToolCall {
            id: String::new(),
            name: "bash".into(),
            arguments: "{}".into(),
        }]);
        let mut tool_msg = ChatMessage::tool_result("", "ok");
        tool_msg.tool_call_id = Some(String::new());

        let (input, _) = OpenAiResponsesProvider::build_input(&[calls_msg, tool_msg]);
        let call = &input[0];
        assert_eq!(call["type"], "function_call");
        assert_ne!(call["call_id"], ""); // strict upstream rejects ""
        let output = &input[1];
        assert_eq!(output["type"], "function_call_output");
        assert_eq!(output["call_id"], call["call_id"]); // paired positionally
    }

    #[test]
    fn build_input_drops_orphan_outputs() {
        // A tool row whose call_id never appeared upstream stays dropped.
        let mut tool_msg = ChatMessage::tool_result("nope", "x");
        tool_msg.tool_call_id = Some("nope".into());
        let (input, _) = OpenAiResponsesProvider::build_input(&[tool_msg]);
        assert!(input.is_empty());
    }

    #[test]
    fn build_input_synthesizes_missing_outputs() {
        // deepseek's Responses shim rejects a `function_call` whose output
        // never landed — "No tool output found for tool call …" (400). The
        // placeholder must sit immediately after the call row, before the
        // next non-output item.
        let mut calls_msg = ChatMessage::assistant("calling");
        calls_msg.tool_calls = Some(vec![
            ToolCall { id: "call_1".into(), name: "bash".into(), arguments: "{}".into() },
            ToolCall { id: "call_2".into(), name: "bash".into(), arguments: "{}".into() },
        ]);
        let mut out1 = ChatMessage::tool_result("call_1", "ok");
        out1.tool_call_id = Some("call_1".into());
        let (input, _) = OpenAiResponsesProvider::build_input(&[
            calls_msg, out1, ChatMessage::assistant("done"),
        ]);
        // assistant text, function_call, function_call, output(call_1),
        // output(call_2 synth), assistant text.
        let types: Vec<&str> = input
            .iter()
            .map(|i| i["type"].as_str().or_else(|| i["role"].as_str()).unwrap_or("?"))
            .collect();
        assert_eq!(types, [
            "assistant", "function_call", "function_call",
            "function_call_output", "function_call_output",
            "assistant",
        ]);
        assert_eq!(input[4]["call_id"], "call_2");
    }

    #[test]
    fn build_input_flushes_missing_at_end() {
        // History ends mid-tools — the trailing call gets a placeholder
        // output so the replayed request stays pair-valid.
        let mut calls_msg = ChatMessage::assistant("calling");
        calls_msg.tool_calls = Some(vec![
            ToolCall { id: "call_z".into(), name: "bash".into(), arguments: "{}".into() },
        ]);
        let (input, _) = OpenAiResponsesProvider::build_input(&[calls_msg]);
        assert_eq!(input.len(), 3);
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_z");
    }

    #[test]
    fn build_input_does_not_flush_on_system_rows() {
        // A mid-block system notice folds into `instructions` — it must not
        // trigger placeholder synthesis, or the real tool row after it
        // would orphan-drop.
        let mut calls_msg = ChatMessage::assistant("calling");
        calls_msg.tool_calls = Some(vec![
            ToolCall { id: "call_1".into(), name: "bash".into(), arguments: "{}".into() },
        ]);
        let mut out1 = ChatMessage::tool_result("call_1", "real output");
        out1.tool_call_id = Some("call_1".into());
        let (input, _) = OpenAiResponsesProvider::build_input(&[
            calls_msg,
            ChatMessage::notice("已插入引导指令"),
            out1,
        ]);
        let outputs: Vec<_> = input
            .iter()
            .filter(|i| i["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0]["output"], "real output");
    }

    #[test]
    fn build_input_blank_output_pairs_with_unanswered_call() {
        // Mixed snapshot: calls kept real ids, but one output row was
        // persisted `tool_call_id: ""`. The blank row must pair with the
        // first UNANSWERED call — not pop an already-paired id off the
        // blank queue and emit a duplicate output for it.
        let mut calls_msg = ChatMessage::assistant("calling");
        calls_msg.tool_calls = Some(vec![
            ToolCall { id: "call_1".into(), name: "bash".into(), arguments: "{}".into() },
            ToolCall { id: "call_2".into(), name: "bash".into(), arguments: "{}".into() },
        ]);
        let mut out1 = ChatMessage::tool_result("call_1", "first");
        out1.tool_call_id = Some("call_1".into());
        let mut out2 = ChatMessage::tool_result("", "second");
        out2.tool_call_id = Some(String::new());

        let (input, _) =
            OpenAiResponsesProvider::build_input(&[calls_msg, out1, out2]);
        let outputs: Vec<_> = input
            .iter()
            .filter(|i| i["type"] == "function_call_output")
            .collect();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0]["call_id"], "call_1");
        assert_eq!(outputs[0]["output"], "first");
        assert_eq!(outputs[1]["call_id"], "call_2");
        assert_eq!(outputs[1]["output"], "second");
    }

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

    #[test]
    fn call_with_bracket_in_args_terminates_on_close_plus_newline() {
        // `]` inside JSON args (`result[j]`) must NOT end the call — only
        // `]` followed by a newline (or held at end) does.
        let mut s = CallStripper::new();
        s.feed("pre\n[call: fuzzy_patch({\"s\":\"result[j] = 1\"})]\npost");
        let calls = s.take_recovered();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "fuzzy_patch");
        assert!(calls[0].arguments.contains("result[j]"));
    }

    #[test]
    fn open_bracket_at_delta_end_is_held() {
        // `[` at the end of a delta is held — a following `call:` doesn't
        // leak as text (the old impl dropped the `[` when the 5-char
        // lookahead came up short).
        let mut s = CallStripper::new();
        assert!(s.feed("hi [").unwrap_or_default().starts_with("hi"));
        s.feed("call: bash({\"c\":1})]\ndone");
        let calls = s.take_recovered();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
    }
}
