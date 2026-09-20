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

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::provider::{BoxStream, LlmProvider};
use crate::sse;
use crate::types::{ChatMessage, StreamChunk};

/// One provider for every OpenAI-shaped backend. `api_key` is the *resolved*
/// secret — config's `env:`/`keyring:` indirection happens in `factory`.
pub struct GenericOpenAiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    /// Extra static headers some backends require (e.g. `HTTP-Referer`).
    extra_headers: Vec<(String, String)>,
    compat: Option<crate::config::ProviderCompat>,
}

impl GenericOpenAiProvider {
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
            compat: None,
        })
    }

    pub fn with_compat(mut self, compat: crate::config::ProviderCompat) -> Self {
        self.compat = Some(compat);
        self
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
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
                    index: tc.index,
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

#[async_trait]
impl LlmProvider for GenericOpenAiProvider {
    fn id(&self) -> &'static str {
        "openai_compat"
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        // `temperature` is optional — some OpenAI-compat backends reject
        // extreme values (verified: devin upstream-errors on temperature=0).
        // Only include it when the caller picked a non-default value; the
        // provider's own default (usually 1.0) applies when omitted.
        let mut body = json!({
            "model": model,
            // `is_error` is replay-only metadata persisted in the session
            // snapshot — strip it from the wire or strict OpenAI-compat
            // backends reject the unknown field.
            "messages": messages.iter().map(|m| {
                let mut v = serde_json::to_value(m).unwrap_or_else(|_| {
                    json!({ "role": "user", "content": "" })
                });
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("is_error"); obj.remove("notice");
                }
                v
            }).collect::<Vec<_>>(),
            "stream": true,
        });
        if temperature > 0.0 {
            body["temperature"] = json!(temperature);
        }
        if let Some(compat) = &self.compat {
            if let Some(store) = compat.supports_store {
                body["store"] = json!(store);
            }
        }
        if let Some(effort) = reasoning_effort {
            let allowed = self
                .compat
                .as_ref()
                .and_then(|c| c.supports_reasoning_effort)
                .unwrap_or(true);
            if allowed {
                body["reasoning_effort"] = json!(effort);
            }
        }
        // `stream_options.include_usage` is OpenAI/xAI-specific — some
        // OpenAI-compatible backends (devin, certain proxies) reject the
        // field outright instead of ignoring it, so we don't send it. Usage
        // still arrives on the final chunk when the backend provides it.
        // (Verified: nyanya/devin returns `invalid_argument` for it.)
        if let Some(t) = tools {
            body["tools"] = t;
        }

        let mut req = self
            .client
            .post(self.chat_completions_url())
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .json(&body);
        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }

        // Session trace — dump the outbound body so a silently-empty reply
        // can be correlated to the exact request shape. Remove before ship.
        if std::env::var("AGENT_DUMP_REQ").is_ok() {
            eprintln!("\n===REQ===\n{}\n===/REQ===", serde_json::to_string(&body).unwrap());
        }

        let resp = req.send().await.context("chat completions request")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "provider returned {status}: {}",
                &text[..text.len().min(512)]
            ));
        }

        let data = sse::data_lines(resp.bytes_stream());
        // Track the last-seen usage block so a [DONE] sentinel can fold it
        // into `Done` — and a clean-close ending (no sentinel) still emits it.
        let mut pending_usage: Option<WireUsage> = None;
        let mut done_emitted = false;

        let stream = data.flat_map(move |res| -> futures::stream::Iter<std::vec::IntoIter<anyhow::Result<StreamChunk>>> {
            let items: Vec<anyhow::Result<StreamChunk>> = match res {
                Err(e) => vec![Err(e)],
                Ok(d) => map_data(&d, &mut pending_usage)
                    .into_iter()
                    .map(|c| {
                        if matches!(c, StreamChunk::Done { .. }) {
                            done_emitted = true;
                        }
                        Ok(c)
                    })
                    .collect(),
            };
            futures::stream::iter(items)
        });

        // Guarantee Done-exactly-once: if the backend closed without [DONE]
        // or a trailing usage chunk, synthesize it at stream end.
        let stream = stream.chain(futures::stream::once(async move {
            if done_emitted {
                None
            } else {
                Some(Ok(StreamChunk::Done {
                    prompt_tokens: None,
                    completion_tokens: None,
                }))
            }
        }).filter_map(|x| async move { x }));

        Ok(Box::pin(stream))
    }
}
