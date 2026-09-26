//! rig-core backed `LlmProvider` — the single provider implementation.
//!
//! rig-core owns the wire layer (HTTP transport, SSE, response parsing,
//! streaming normalization) for every `ProviderKind`; this module keeps the
//! kernel/UI contract (`ChatMessage`, `StreamChunk`, `ToolCallAssembler`)
//! and the policy layer (`Sampler`, `EgressMasker`) untouched above it.
//!
//! What still lives here — the parts rig deliberately leaves to the caller:
//!
//! - **Request-side dialects** (`ProviderCompat`): OpenAI-compatible shims
//!   disagree on field names (`max_tokens` vs `max_completion_tokens` vs
//!   `max_output_tokens`), thinking-parameter shapes (`reasoning_effort`,
//!   `reasoning.effort`, `thinking`, `enable_thinking`, `chat_template_kwargs`,
//!   …), system-role spelling (`system` vs `developer`), and replay rules
//!   (`reasoning_content`, tool-result `name`). Additive fields go through
//!   `additional_params`; mutations of the assembled body happen inside
//!   [`CompatExt`]'s `finalize_request_body` hook.
//! - **Message pairing repair**: history can carry assistant `tool_calls`
//!   whose results never landed (a killed stream) or orphaned results — both
//!   are hard 400s on strict backends, so the converter drops orphans and
//!   synthesizes placeholder results for dangling calls.
//! - **Stream-layer shims**: `HarmonyStripper` (harmony/gpt-oss control
//!   tokens leaking into text/reasoning) and `CallStripper` (`[call: …]`
//!   text-protocol escape hatch) run on rig's normalized deltas, unchanged.
//! - **ProviderCompat-driven usage**: `CompatUsage` reads both OpenAI's
//!   `prompt_tokens_details.cached_tokens` and the DeepSeek-style
//!   `prompt_cache_hit_tokens` dialect fields so `cached_tokens` keeps
//!   working across gateways.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;

use async_trait::async_trait;
use futures::StreamExt;
use rig_core::client::{self, BearerAuth, Capable, CompletionClient, Nothing};
use rig_core::completion::{CompletionModel, CompletionRequest, ToolDefinition};
use rig_core::completion::request::CompletionError;
use rig_core::message::{
    AssistantContent, DocumentSourceKind, Image as RigImage, ImageMediaType,
    Message as RigMessage, Reasoning, ReasoningContent, Text as RigText,
    ToolCall as RigToolCall, ToolCallId, ToolFunction, ToolResult as RigToolResult,
    ToolResultContent, UserContent,
};
use rig_core::providers::{anthropic, gemini, openai};
use rig_core::streaming::{StreamedAssistantContent, StreamingCompletionResponse, ToolCallDeltaContent};
use rig_core::http_client::{self, HttpClientExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::config::{ProviderCompat, ProviderConfig, ProviderKind};
use crate::provider::{BoxStream, Capabilities, LlmProvider, ModelParams};
use crate::types::{ChatMessage, HarmonyStripper, ImageRef, NoticeKind, Role, StreamChunk};

/// Anthropic rejects a request without `max_tokens`; adapters defaulted 32768.
const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 32_768;

// ================================================================
// OpenAI-compatible provider extension carrying `ProviderCompat`
// ================================================================

/// Extension type for the generic OpenAI chat-completions client. Carries the
/// per-provider `ProviderCompat` so dialect hooks (`finalize_request_body`)
/// can apply deployment-specific wire mutations rig knows nothing about.
#[derive(Debug, Clone, Default)]
struct CompatExt {
    compat: Option<ProviderCompat>,
}

#[derive(Debug, Clone, Default)]
struct CompatExtBuilder;

impl client::Provider for CompatExt {
    type Builder = CompatExtBuilder;
    const VERIFY_PATH: &'static str = "/models";
}

impl client::DebugExt for CompatExt {}

impl client::ProviderBuilder for CompatExtBuilder {
    type Extension<H: HttpClientExt> = CompatExt;
    // The provider's `api_key` travels as an `Authorization` header we insert
    // ourselves (so `auth_header = false` can truly omit it); the builder's
    // own key slot stays `Nothing`.
    type ApiKey = Nothing;
    const BASE_URL: &'static str = "https://api.openai.com/v1";

    fn build<H: HttpClientExt>(
        _builder: &client::ClientBuilder<Self, Self::ApiKey, H>,
    ) -> http_client::Result<CompatExt> {
        Ok(CompatExt::default())
    }
}

impl<H> client::Capabilities<H> for CompatExt {
    type Completion = Capable<openai::completion::GenericCompletionModel<CompatExt, H>>;
    type Embeddings = Nothing;
    type Transcription = Nothing;
    type ModelListing = Nothing;
    type Rerank = Nothing;
}

impl openai::completion::OpenAICompatibleProvider for CompatExt {
    const PROVIDER_NAME: &'static str = "openai-compatible";
    const REQUEST_ID_HEADER: Option<&'static str> = Some("x-request-id");

    type StreamingUsage = CompatUsage;
    type Response = openai::completion::CompletionResponse;

    /// Wire-level dialect surgery: runs on the fully serialized request body
    /// after rig merges streaming params, immediately before send.
    fn finalize_request_body(&self, body: &mut Value) -> Result<(), CompletionError> {
        compat_body_patches(body, self.compat.as_ref());
        Ok(())
    }
}

/// Streaming usage for OpenAI-compatible wires — OpenAI's standard shape plus
/// the cache fields DeepSeek-style providers add (`prompt_cache_hit_tokens`
/// / `prompt_cache_miss_tokens`), so cached-token accounting survives
/// gateways that report it either way.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
struct CompatUsage {
    prompt_tokens: u64,
    completion_tokens: Option<u64>,
    total_tokens: u64,
    prompt_tokens_details: Option<CompatPromptDetails>,
    prompt_cache_hit_tokens: Option<u64>,
    prompt_cache_miss_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
struct CompatPromptDetails {
    cached_tokens: Option<u64>,
}

impl From<CompatUsage> for rig_core::completion::Usage {
    fn from(u: CompatUsage) -> Self {
        let cached = u
            .prompt_tokens_details
            .and_then(|d| d.cached_tokens)
            .or(u.prompt_cache_hit_tokens)
            .unwrap_or(0);
        rig_core::completion::Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u
                .completion_tokens
                .unwrap_or_else(|| u.total_tokens.saturating_sub(u.prompt_tokens)),
            total_tokens: u.total_tokens,
            cached_input_tokens: cached,
            ..Default::default()
        }
    }
}

// ================================================================
// RigProvider
// ================================================================

/// Which rig wire a provider speaks. Mirrors `ProviderKind` one-to-one —
/// kept as a separate enum so the bridge never has to depend on config
/// serialization details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireKind {
    Compat,
    Responses,
    Anthropic,
    Gemini,
}

impl WireKind {
    fn from(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::OpenaiCompat => Self::Compat,
            ProviderKind::OpenaiResponses => Self::Responses,
            ProviderKind::Anthropic => Self::Anthropic,
            ProviderKind::Gemini => Self::Gemini,
        }
    }
}

/// A rig completion-model handle, one enum arm per wire.
enum RigModel {
    Compat(openai::completion::GenericCompletionModel<CompatExt>),
    Responses(openai::responses_api::ResponsesCompletionModel),
    Anthropic(anthropic::completion::CompletionModel),
    Gemini(gemini::completion::CompletionModel),
}

/// Provider backed by rig-core. Holds only client-construction inputs —
/// the rig client is built per `chat_stream` call so session-affinity
/// headers and the effective (model-merged) `ProviderCompat` reach each
/// request.
pub struct RigProvider {
    id: &'static str,
    base_url: String,
    api_key: String,
    auth_header: bool,
    headers: HashMap<String, String>,
    kind: WireKind,
    compat: Option<ProviderCompat>,
    http: reqwest::Client,
    caps: Capabilities,
}

impl RigProvider {
    /// Build from a resolved provider config. `api_key` is the already-
    /// resolved secret (empty when `auth_header` is false or the provider
    /// needs no key).
    pub fn new(cfg: &ProviderConfig, api_key: String, http: reqwest::Client) -> anyhow::Result<Self> {
        let kind = WireKind::from(cfg.kind);
        let caps = match kind {
            WireKind::Compat | WireKind::Responses => Capabilities::all(),
            WireKind::Anthropic => Capabilities {
                native_tool_calls: true,
                reasoning: true,
                temperature_with_reasoning: false,
                vision: true,
            },
            WireKind::Gemini => Capabilities {
                native_tool_calls: true,
                reasoning: true,
                temperature_with_reasoning: true,
                vision: true,
            },
        };
        Ok(Self {
            id: match kind {
                WireKind::Compat => "openai_compat",
                WireKind::Responses => "openai_responses",
                WireKind::Anthropic => "anthropic",
                WireKind::Gemini => "gemini",
            },
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            auth_header: cfg.auth_header.unwrap_or(true),
            headers: cfg.headers.clone(),
            kind,
            compat: cfg.compat.clone(),
            http,
            caps,
        })
    }

    /// Provider-level `ProviderCompat` is on the ext that goes through
    /// `with_ext`, so dialect hooks see the model-merged values only if we
    /// rebuild — which we do per call anyway.
    fn build_model(
        &self,
        model: &str,
        compat: &ProviderCompat,
        thinking_on: bool,
    ) -> anyhow::Result<RigModel> {
        let mut headers = HeaderMap::new();
        for (k, v) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, value);
            }
        }
        let http = self.http.clone();
        Ok(match self.kind {
            WireKind::Compat => {
                if self.auth_header && !self.api_key.is_empty() {
                    if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                    {
                        headers
                            .entry(reqwest::header::AUTHORIZATION)
                            .or_insert(v);
                    }
                }
                let b = client::Client::<CompatExt>::builder()
                    .base_url(&self.base_url)
                    .http_headers(headers)
                    .http_client(http);
                let client = b
                    .api_key(Nothing)
                    .build()
                .map_err(|e| anyhow::anyhow!("openai-compatible client: {e}"))?
                    .with_ext(CompatExt {
                        compat: Some(compat.clone()),
                    });
                let model = client.completion_model(model.to_string());
                RigModel::Compat(model)
            }
            WireKind::Responses => {
                let b = openai::Client::builder()
                    .base_url(&self.base_url)
                    .http_headers(headers)
                    .http_client(http);
                let client = if self.auth_header && !self.api_key.is_empty() {
                    b.api_key(BearerAuth::from(self.api_key.clone())).build()
                } else {
                    // rig requires an ApiKey type — an empty bearer still
                    // emits an `Authorization: Bearer ` header.
                    b.api_key(BearerAuth::from(String::new())).build()
                }
                .map_err(|e| anyhow::anyhow!("openai-responses client: {e}"))?;
                // Shims that reject the top-level `instructions` field get
                // system messages inside `input` instead.
                let client = if compat
                    .supports_developer_role
                    .map(|v| !v)
                    .unwrap_or(false)
                {
                    client.with_system_instructions_as_messages()
                } else {
                    client
                };
                let model = client.completion_model(model.to_string());
                RigModel::Responses(model)
            }
            WireKind::Anthropic => {
                let b = anthropic::Client::builder()
                    .base_url(&self.base_url)
                    .http_headers(headers)
                    .http_client(http);
                let b = if self.auth_header && !self.api_key.is_empty() {
                    b.api_key(anthropic::client::AnthropicKey::from(self.api_key.clone()))
                } else {
                    b.api_key(anthropic::client::AnthropicKey::from(String::new()))
                };
                // Interleaved thinking is still behind the beta header —
                // send it only when a thinking block will actually ride the
                // request (resolved budget exists), matching the retired
                // adapter.
                let b = if thinking_on {
                    b.anthropic_beta("interleaved-thinking-2025-05-14")
                } else {
                    b
                };
                let client = b
                    .build()
                .map_err(|e| anyhow::anyhow!("anthropic client: {e}"))?;
                let model = client.completion_model(model.to_string());
                RigModel::Anthropic(model)
            }
            WireKind::Gemini => {
                let b = gemini::Client::builder()
                    .base_url(&self.base_url)
                    .http_headers(headers)
                    .http_client(http);
                let client = if self.auth_header && !self.api_key.is_empty() {
                    b.api_key(gemini::client::GeminiApiKey::from(self.api_key.clone())).build()
                } else {
                    b.api_key(gemini::client::GeminiApiKey::from(String::new())).build()
                }
                .map_err(|e| anyhow::anyhow!("gemini client: {e}"))?;
                let model = client.completion_model(model.to_string());
                RigModel::Gemini(model)
            }
        })
    }
}

#[async_trait]
impl LlmProvider for RigProvider {
    fn id(&self) -> &'static str {
        self.id
    }

    fn capabilities(&self) -> Capabilities {
        self.caps
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
        params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        // Model-level `compat` merged over the provider's — `maxTokensField`
        // and `supportsReasoningEffort` are commonly declared per model.
        let compat = params.compat_with(self.compat.as_ref());
        let effort = reasoning_effort.filter(|_| {
            compat.supports_reasoning_effort.unwrap_or(true)
        });

        // ---------- messages ----------
        let chat_history = to_rig_messages(messages, self.kind, &compat);

        // ---------- request ----------
        // Temperature is optional: several backends reject extreme values
        // (devin upstream-errors on temperature=0), and Anthropic rejects it
        // while thinking is enabled — only send when meaningful.
        let temperature = if temperature > 0.0
            && !(effort.is_some() && !self.caps.temperature_with_reasoning)
        {
            Some(temperature as f64)
        } else {
            None
        };

        // Output cap. Compat wires disagree on the field name, so the cap is
        // pushed through `additional_params` under whatever name the dialect
        // declares (`maxTokensField` — `max_completion_tokens` default).
        let mut additional = Map::new();
        let max_tokens_field = compat.max_tokens_field_name();
        let mut max_tokens = params.max_tokens.map(|v| v as u64);
        // Claude requires `1024 ≤ thinking.budget_tokens < max_tokens` —
        // resolve the budget against the real cap once so both the request
        // body and the `anthropic-beta` header see the same outcome.
        let anthropic_budget = if self.kind == WireKind::Anthropic {
            anthropic_effective_budget(
                effort,
                params.max_tokens.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS),
            )
        } else {
            None
        };
        match self.kind {
            WireKind::Compat => {
                if let Some(cap) = max_tokens.take() {
                    additional.insert(max_tokens_field.to_string(), json!(cap));
                }
                if let Some(store) = compat.supports_store {
                    additional.insert("store".into(), json!(store));
                }
                if let Some(eff) = effort {
                    apply_thinking(&mut additional, &compat, eff);
                }
                if let Some(v) = &compat.open_router_routing {
                    additional.insert("provider".into(), v.clone());
                }
                if let Some(v) = &compat.vercel_gateway_routing {
                    additional.insert("gateway".into(), v.clone());
                }
            }
            WireKind::Responses => {
                // Responses API caps at `max_output_tokens`.
                if let Some(cap) = max_tokens.take() {
                    additional.insert("max_output_tokens".into(), json!(cap));
                }
                if let Some(eff) = effort {
                    additional.insert("reasoning".into(), json!({ "effort": eff }));
                }
            }
            WireKind::Anthropic => {
                let cap = max_tokens
                    .take()
                    .map(|v| v as u32)
                    .unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS);
                max_tokens = Some(cap as u64);
                if let Some(budget) = anthropic_budget {
                    additional.insert(
                        "thinking".into(),
                        json!({ "type": "enabled", "budget_tokens": budget }),
                    );
                }
            }
            WireKind::Gemini => {
                if let Some(budget) = effort.and_then(gemini_thinking_budget) {
                    additional.insert(
                        "generationConfig".into(),
                        json!({ "thinkingConfig": { "thinkingBudget": budget } }),
                    );
                }
            }
        }
        // `samplingParams` merges last so its keys win (pi semantics).
        if let Some(extra) = params.sampling_params.as_ref().and_then(|v| v.as_object()) {
            for (k, v) in extra {
                additional.insert(k.clone(), v.clone());
            }
        }

        let request = CompletionRequest {
            model: None,
            preamble: None,
            chat_history,
            documents: Vec::new(),
            tools: tool_defs(tools),
            temperature,
            max_tokens,
            tool_choice: None,
            additional_params: (!additional.is_empty())
                .then(|| Value::Object(additional)),
            output_schema: None,
            record_telemetry_content: false,
        };

        let rig_model = self.build_model(model, &compat, anthropic_budget.is_some())?;
        let rig_stream: StreamingCompletionResponse = match &rig_model {
            RigModel::Compat(m) => m.stream(request).await,
            RigModel::Responses(m) => m.stream(request).await,
            RigModel::Anthropic(m) => m.stream(request).await,
            RigModel::Gemini(m) => m.stream(request).await,
        }
        .map_err(|e| anyhow::anyhow!("{}", map_completion_error(&e)))?;

        Ok(map_rig_stream(rig_stream, self.kind))
    }
}

// ================================================================
// Dialect application — compat wire
// ================================================================

/// `reasoning_effort` → the dialect field a provider declares. `openai` (the
/// default) sends `reasoning_effort`; other formats express the same intent
/// through their own shape. Ported verbatim from the retired adapter.
fn apply_thinking(body: &mut Map<String, Value>, compat: &ProviderCompat, effort: &str) {
    match compat.thinking_format_name() {
        // `reasoning: { effort }` — OpenRouter's shape.
        "openrouter" => {
            body.insert("reasoning".into(), json!({ "effort": effort }));
        }
        // `reasoning: { enabled }` — Together; `reasoning_effort` too when the
        // provider also declares support for it.
        "together" => {
            body.insert(
                "reasoning".into(),
                json!({ "enabled": effort != "none" && effort != "off" }),
            );
            if compat.supports_reasoning_effort == Some(true) {
                body.insert("reasoning_effort".into(), json!(effort));
            }
        }
        // Top-level `enable_thinking` — Qwen / DashScope.
        "qwen" => {
            body.insert(
                "enable_thinking".into(),
                json!(effort != "none" && effort != "off"),
            );
        }
        // `chat_template_kwargs` — local Qwen-compatible servers.
        "qwen-chat-template" => {
            body.insert(
                "chat_template_kwargs".into(),
                json!({
                    "enable_thinking": effort != "none" && effort != "off",
                    "preserve_thinking": true,
                }),
            );
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
                resolve_thinking_var(v, effort);
            }
            kwargs.retain(|_, v| !v.is_null());
            body.insert("chat_template_kwargs".into(), Value::Object(kwargs));
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
                resolve_thinking_var(v, effort);
            }
            args.retain(|_, v| !v.is_null());
            body.insert("chat_template_args".into(), Value::Object(args));
        }
        // DeepSeek / ZAI / ant-ling `thinking: { type }`; bare-string and the
        // OpenAI fallback.
        "deepseek" | "zai" | "ant-ling" => {
            let enabled = effort != "none" && effort != "off";
            body.insert(
                "thinking".into(),
                json!({ "type": if enabled { "enabled" } else { "disabled" } }),
            );
        }
        "string-thinking" => {
            body.insert("thinking".into(), json!(effort));
        }
        _ => {
            body.insert("reasoning_effort".into(), json!(effort));
        }
    }
}

/// Substitute a `{ "$var": "thinking.…" }` placeholder with the value of that
/// thinking knob (pi's `chatTemplateKwargs`/`chatTemplateArgs` syntax).
fn resolve_thinking_var(v: &mut Value, effort: &str) {
    let Some(obj) = v.as_object() else {
        return;
    };
    let Some(var) = obj.get("$var").and_then(|x| x.as_str()) else {
        return;
    };
    let enabled = effort != "none" && effort != "off";
    // `omitWhenOff` drops the key entirely when thinking is disabled — the
    // caller removes the resulting Null.
    let omit = obj
        .get("omitWhenOff")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    *v = match var {
        "thinking.enabled" => {
            if omit && !enabled {
                Value::Null
            } else {
                json!(enabled)
            }
        }
        "thinking.effort" => json!(effort),
        _ => return,
    };
}

/// `reasoning_effort` → Anthropic `thinking.budget_tokens`.
fn anthropic_thinking_budget(effort: &str) -> Option<u32> {
    match effort {
        "low" => Some(2_048),
        "medium" => Some(8_192),
        "high" => Some(16_384),
        "max" => Some(28_672),
        _ => None,
    }
}

/// Anthropic `thinking.budget_tokens` vs the real output cap — Claude
/// hard-rejects `budget_tokens ≥ max_tokens` and requires `≥ 1024`. The
/// effort-table budgets assume the 32k default cap; a smaller declared cap
/// clamps into the valid window, and a cap that can't fit a legal budget
/// drops the thinking block entirely (no block, no beta header).
fn anthropic_effective_budget(effort: Option<&str>, cap: u32) -> Option<u32> {
    effort
        .and_then(anthropic_thinking_budget)
        .map(|b| b.min(cap.saturating_sub(1)))
        .filter(|b| *b >= 1024)
}

/// `reasoning_effort` → Gemini `thinkingConfig.thinkingBudget` — `-1` lets the
/// model pick its own budget.
fn gemini_thinking_budget(effort: &str) -> Option<i64> {
    match effort {
        "low" => Some(1_024),
        "medium" => Some(8_192),
        "high" => Some(24_576),
        "max" => Some(-1),
        _ => None,
    }
}

/// Body-level dialect surgery for OpenAI-compatible wires — the mutations
/// that `additional_params` cannot express because they rewrite fields rig
/// already serialized.
fn compat_body_patches(body: &mut Value, compat: Option<&ProviderCompat>) {
    let Some(compat) = compat else {
        return;
    };
    let Some(map) = body.as_object_mut() else {
        return;
    };

    // `stream_options.include_usage` is an OpenAI extension — several shims
    // (devin, proxies) reject the field outright, so it is only sent when the
    // deployment explicitly opts in. Usage still arrives on the final chunk.
    if !compat.supports_usage_in_streaming.unwrap_or(false) {
        map.remove("stream_options");
    }

    // Anthropic-flavoured prompt caching markers on an OpenAI wire.
    if compat
        .cache_control_format
        .as_deref()
        .map(|f| f.eq_ignore_ascii_case("anthropic"))
        .unwrap_or(false)
    {
        apply_cache_control_markers(map);
    }

    let developer_role = compat.developer_role();
    let replay_empty_reasoning = compat
        .requires_reasoning_content_on_assistant_messages
        .unwrap_or(false);
    let tool_result_name = compat.requires_tool_result_name.unwrap_or(false);
    let effort_on = map.contains_key("reasoning_effort")
        || map.contains_key("reasoning")
        || map.contains_key("thinking")
        || map.contains_key("enable_thinking")
        || map.contains_key("chat_template_kwargs")
        || map.contains_key("chat_template_args");

    let Some(messages) = map.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    // `requiresToolResultName`: tool messages need `name` — recover it from
    // the matching assistant `tool_calls` entry.
    let mut call_names: HashMap<String, String> = HashMap::new();
    if tool_result_name {
        for m in messages.iter() {
            if m.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            if let Some(calls) = m.get("tool_calls").and_then(Value::as_array) {
                for c in calls {
                    let id = c.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = c
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !id.is_empty() {
                        call_names.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }
    }

    for m in messages.iter_mut() {
        let Some(msg) = m.as_object_mut() else {
            continue;
        };
        let role = msg.get("role").and_then(Value::as_str).unwrap_or_default();
        match role {
            "system" if developer_role => {
                msg.insert("role".into(), json!("developer"));
            }
            "assistant" => {
                // `requiresReasoningContentOnAssistantMessages` — the key must
                // exist (empty is fine) on every assistant replay while
                // reasoning is enabled.
                if effort_on
                    && replay_empty_reasoning
                    && !msg.contains_key("reasoning_content")
                {
                    msg.insert("reasoning_content".into(), json!(""));
                }
            }
            "tool" if tool_result_name => {
                if !msg.contains_key("name") {
                    let id = msg
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if let Some(name) = call_names.get(id) {
                        msg.insert("name".into(), json!(name));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Insert `cache_control: {"type": "ephemeral"}` on the last content part of
/// the last system and last user message — the Anthropic cache-breakpoint
/// convention ported onto compatible wires.
fn apply_cache_control_markers(map: &mut Map<String, Value>) {
    let Some(messages) = map.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let mark = json!({ "type": "ephemeral" });
    let mut last_system: Option<usize> = None;
    let mut last_user: Option<usize> = None;
    for (i, m) in messages.iter().enumerate() {
        match m.get("role").and_then(Value::as_str) {
            Some("system") | Some("developer") => last_system = Some(i),
            Some("user") => last_user = Some(i),
            _ => {}
        }
    }
    for idx in [last_system, last_user].into_iter().flatten() {
        let Some(m) = messages.get_mut(idx).and_then(Value::as_object_mut) else {
            continue;
        };
        match m.get_mut("content") {
            Some(Value::Array(parts)) => {
                if let Some(Value::Object(part)) = parts.last_mut() {
                    part.entry("cache_control").or_insert_with(|| mark.clone());
                }
            }
            Some(Value::Object(part)) => {
                part.entry("cache_control").or_insert_with(|| mark.clone());
            }
            Some(Value::String(_)) => {
                // String content can't carry the marker — wrap into parts.
                let text = m.get("content").and_then(Value::as_str).unwrap_or_default().to_string();
                m.insert(
                    "content".into(),
                    json!([{ "type": "text", "text": text, "cache_control": mark }]),
                );
            }
            _ => {}
        }
    }
}

// ================================================================
// Message conversion — ChatMessage → rig Message (+ pairing repair)
// ================================================================

/// Convert kernel history into rig messages. `kind` selects the small set of
/// wire-shape differences rig's own conversion can't express on our behalf
/// (image-in-tool-result support, reasoning replay policy).
fn to_rig_messages(
    messages: &[ChatMessage],
    kind: WireKind,
    compat: &ProviderCompat,
) -> Vec<RigMessage> {
    // Pairing ledger — assistant call ids that still need results. Orphan
    // results are dropped; dangling calls get a placeholder at the end.
    // Declared call ids still awaiting a result — HashSet membership plus a
    // Vec for deterministic placeholder order (the old Vec's contains/retain
    // was O(n²) over long tool-call histories).
    let mut pending: HashSet<String> = HashSet::new();
    let mut pending_order: Vec<String> = Vec::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut synth_seq = 0usize;
    let mut out: Vec<RigMessage> = Vec::new();
    let mut pending_tool_images: Vec<RigImage> = Vec::new();
    let thinking_as_text = compat.requires_thinking_as_text.unwrap_or(false);
    let assistant_after_tool = compat
        .requires_assistant_after_tool_result
        .unwrap_or(false);

    /// flush tool-result images into a user message (openai-chat wire cannot
    /// carry them inside the tool result).
    macro_rules! flush_tool_images {
        () => {
            if !pending_tool_images.is_empty() {
                let mut parts = vec![UserContent::Text(RigText::new(
                    "(screenshots from the tool results above)",
                ))];
                for img in pending_tool_images.drain(..) {
                    parts.push(UserContent::Image(img));
                }
                out.push(RigMessage::User { content: parts });
            }
        };
    }

    for m in messages {
        // UI-only rows — render metadata, never context.
        if m.role == Role::System
            && matches!(m.notice, Some(NoticeKind::Compacted) | Some(NoticeKind::Plan))
        {
            continue;
        }
        match m.role {
            Role::System => {
                let text = m.content.clone().unwrap_or_default();
                if m.notice == Some(NoticeKind::CompactedMemory) {
                    // Historical context, not a live instruction — user
                    // privilege so a summary can't become a system prompt.
                    flush_tool_images!();
                    out.push(RigMessage::User {
                        content: vec![UserContent::Text(RigText::new(text))],
                    });
                } else {
                    out.push(RigMessage::System { content: text });
                }
            }
            Role::User => {
                flush_tool_images!();
                let mut parts: Vec<UserContent> = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        parts.push(UserContent::Text(RigText::new(t.clone())));
                    }
                }
                for img in &m.images {
                    match to_rig_image(img, kind) {
                        Some(i) => parts.push(UserContent::Image(i)),
                        None => parts.push(UserContent::Text(RigText::new(format!(
                            "(image unavailable: {})",
                            img.path.display()
                        )))),
                    }
                }
                if !parts.is_empty() {
                    out.push(RigMessage::User { content: parts });
                }
            }
            Role::Assistant => {
                flush_tool_images!();
                let mut content: Vec<AssistantContent> = Vec::new();
                if let Some(t) = &m.content {
                    if !t.is_empty() {
                        content.push(AssistantContent::Text(RigText::new(t.clone())));
                    }
                }
                if let Some(reasoning) = &m.reasoning {
                    if !reasoning.is_empty() {
                        match kind {
                            // Native replay: rig serializes these back as
                            // `reasoning_content` / reasoning items.
                            WireKind::Compat | WireKind::Responses => {
                                if thinking_as_text {
                                    content.push(AssistantContent::Text(RigText::new(
                                        reasoning.clone(),
                                    )));
                                } else {
                                    content.push(AssistantContent::Reasoning(Reasoning {
                                        id: None,
                                        content: vec![ReasoningContent::Text {
                                            text: reasoning.clone(),
                                            signature: None,
                                        }],
                                    }));
                                }
                            }
                            // Anthropic/Gemini replay dropped reasoning in
                            // the retired adapters — keep that behaviour.
                            WireKind::Anthropic | WireKind::Gemini => {}
                        }
                    }
                }
                for tc in m.tool_calls.iter().flatten() {
                    let id = if tc.id.is_empty() {
                        synth_seq += 1;
                        format!("call_synth_{synth_seq}")
                    } else {
                        tc.id.clone()
                    };
                    names.insert(id.clone(), tc.name.clone());
                    pending.insert(id.clone());
                    pending_order.push(id.clone());
                    content.push(AssistantContent::ToolCall(RigToolCall {
                        id: ToolCallId::new_or_mint(id),
                        provider: None,
                        function: ToolFunction {
                            name: tc.name.clone(),
                            arguments: serde_json::from_str(&tc.arguments)
                                .unwrap_or_else(|_| Value::String(tc.arguments.clone())),
                        },
                        signature: None,
                        additional_params: None,
                    }));
                }
                if !content.is_empty() {
                    out.push(RigMessage::Assistant {
                        id: None,
                        content,
                    });
                }
            }
            Role::Tool => {
                let call_id = m.tool_call_id.clone().unwrap_or_default();
                if !call_id.is_empty() && pending.remove(&call_id) {
                    let mut parts: Vec<ToolResultContent> = Vec::new();
                    if let Some(t) = &m.content {
                        if !t.is_empty() {
                            parts.push(ToolResultContent::Text(RigText::new(t.clone())));
                        }
                    }
                    for img in &m.images {
                        match to_rig_image(img, kind) {
                            Some(i) => {
                                if kind == WireKind::Compat {
                                    // OpenAI chat-completions rejects images
                                    // inside tool results — they ride the
                                    // following user message instead.
                                    pending_tool_images.push(i);
                                } else {
                                    parts.push(ToolResultContent::Image(i));
                                }
                            }
                            None => parts.push(ToolResultContent::Text(RigText::new(
                                format!("(image unavailable: {})", img.path.display()),
                            ))),
                        }
                    }
                    if parts.is_empty() {
                        parts.push(ToolResultContent::Text(RigText::new("")));
                    }
                    let name = names.get(&call_id).cloned().unwrap_or_default();
                    out.push(RigMessage::User {
                        content: vec![UserContent::ToolResult(RigToolResult {
                            call: ToolCallId::new_or_mint(call_id),
                            provider: None,
                            name,
                            content: parts,
                        })],
                    });
                    if assistant_after_tool {
                        out.push(RigMessage::Assistant {
                            id: None,
                            content: vec![AssistantContent::Text(RigText::new(""))],
                        });
                    }
                }
                // Unknown / already-covered tool result ids — drop the orphan.
            }
        }
    }

    flush_tool_images!();

    // Dangling assistant calls still need a result — an interrupted stream
    // or a reconstructed session can leave the pair half-written, and strict
    // backends answer that with a hard 400.
    for id in pending_order.iter().filter(|i| pending.contains(i.as_str())) {
        out.push(RigMessage::User {
            content: vec![UserContent::ToolResult(RigToolResult {
                call: ToolCallId::new_or_mint(id.clone()),
                provider: None,
                name: names.get(id).cloned().unwrap_or_default(),
                content: vec![ToolResultContent::Text(RigText::new(
                    "(missing tool result in replayed history — call interrupted)",
                ))],
            })],
        });
    }

    merge_consecutive(out)
}

/// Anthropic requires strict user/assistant alternation; consecutive same-
/// role messages merge their content lists (system messages keep position —
/// every wire treats them separately).
fn merge_consecutive(messages: Vec<RigMessage>) -> Vec<RigMessage> {
    let mut out: Vec<RigMessage> = Vec::new();
    for m in messages {
        match (out.last_mut(), &m) {
            (
                Some(RigMessage::User { content: prev }),
                RigMessage::User { content },
            ) => prev.extend(content.iter().cloned()),
            (
                Some(RigMessage::Assistant { content: prev, .. }),
                RigMessage::Assistant { content, .. },
            ) => prev.extend(content.iter().cloned()),
            _ => out.push(m),
        }
    }
    out
}

/// ChatMessage image → rig image part. For Anthropic/Gemini-style wires the
/// `data:` URL is split into base64 + media type; compat/responses wires take
/// the URL as-is.
fn to_rig_image(img: &ImageRef, kind: WireKind) -> Option<RigImage> {
    use rig_core::message::MimeType;
    let url = img.data_url()?;
    let media_type = ImageMediaType::from_mime_type(&img.media_type);
    match kind {
        WireKind::Compat | WireKind::Responses => Some(RigImage {
            data: DocumentSourceKind::url(&url),
            media_type,
            detail: None,
            additional_params: None,
        }),
        WireKind::Anthropic | WireKind::Gemini => {
            let (_, data) = url.split_once(";base64,")?;
            Some(RigImage {
                data: DocumentSourceKind::base64(data),
                media_type,
                detail: None,
                additional_params: None,
            })
        }
    }
}

// ================================================================
// Tool definitions
// ================================================================

/// Kernel tools arrive as a provider-agnostic JSON array (OpenAI-shaped
/// `[{type:"function",function:{…}}]` or bare `{name,description,parameters}`)
/// — normalize onto rig's `ToolDefinition`. `input_schema`/`inputSchema` are
/// renamed to `parameters`, matching what the retired adapters did.
fn tool_defs(raw: Option<Value>) -> Vec<ToolDefinition> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let arr = match raw {
        Value::Array(a) => a,
        other => vec![other],
    };
    let mut out = Vec::new();
    for t in arr {
        let f = t.get("function").unwrap_or(&t).clone();
        let Some(name) = f.get("name").and_then(Value::as_str) else {
            continue;
        };
        let mut params = f
            .get("parameters")
            .or_else(|| f.get("input_schema"))
            .or_else(|| f.get("inputSchema"))
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        // Non-object parameters can't round-trip through the wires — wrap.
        if !params.is_object() {
            params = json!({"type": "object", "properties": {}});
        }
        out.push(ToolDefinition {
            name: name.to_string(),
            description: f
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            parameters: params,
        });
    }
    out
}

// ================================================================
// Stream mapping — StreamedAssistantContent → StreamChunk
// ================================================================

/// Per-stream mapping state: stripper pipelines and the rig `internal_call_id`
/// → slot-index ledger the assembler counts on.
struct MapState {
    text: HarmonyStripper,
    reasoning: HarmonyStripper,
    /// `[call: …]` text-protocol recovery (OpenAI wires only).
    call_stripper: Option<CallStripper>,
    /// internal_call_id → slot index, in first-seen order.
    slots: HashMap<String, usize>,
    slot_saw_deltas: HashSet<usize>,
    next_slot: usize,
    /// reasoning delta correlator ids seen — a complete `Reasoning` block for
    /// a known id supersedes its deltas and must not double-emit.
    reasoning_ids: HashSet<String>,
    done: bool,
    /// Swallow mid-stream JSON/decode errors (shims leaking non-JSON framing
    /// frames — the data-frame case the retired adapter absorbed).
    swallow_decode_errors: bool,
    /// Total streamed text chars — feeds CallStripper's detection heuristic
    /// on OpenAI wires.
    text_chars: usize,
}

impl MapState {
    fn new(kind: WireKind) -> Self {
        Self {
            text: HarmonyStripper::new(),
            reasoning: HarmonyStripper::new(),
            call_stripper: matches!(kind, WireKind::Compat | WireKind::Responses)
                .then(CallStripper::new),
            slots: HashMap::new(),
            slot_saw_deltas: HashSet::new(),
            next_slot: 0,
            reasoning_ids: HashSet::new(),
            done: false,
            swallow_decode_errors: matches!(kind, WireKind::Compat | WireKind::Responses),
            text_chars: 0,
        }
    }

    fn slot_for(&mut self, internal: &str) -> usize {
        if let Some(s) = self.slots.get(internal) {
            return *s;
        }
        let s = self.next_slot;
        self.next_slot += 1;
        self.slots.insert(internal.to_string(), s);
        s
    }

    fn map_one(&mut self, item: Result<StreamedAssistantContent, CompletionError>) -> Vec<StreamChunk> {
        let mut out = Vec::new();
        match item {
            Ok(StreamedAssistantContent::Text(t)) => {
                let delta = t.text;
                if delta.is_empty() {
                    return out;
                }
                if let Some(stripper) = self.call_stripper.as_mut() {
                    // Text-protocol echo — holds possible `[call:` fragments,
                    // recovers completed calls into real ToolCalls.
                    if let Some(safe) = stripper.feed(&delta) {
                        if let Some(clean) = self.text.feed(&safe) {
                            self.text_chars += clean.chars().count();
                            out.push(StreamChunk::ContentDelta(clean));
                        }
                    }
                    for c in stripper.take_recovered() {
                        let slot = self.next_slot;
                        self.next_slot += 1;
                        out.push(StreamChunk::ToolCallDelta {
                            slot,
                            id: Some(c.id),
                            name: Some(c.name),
                            args_delta: c.arguments,
                        });
                    }
                } else if let Some(clean) = self.text.feed(&delta) {
                    self.text_chars += clean.chars().count();
                    out.push(StreamChunk::ContentDelta(clean));
                }
            }
            Ok(StreamedAssistantContent::ReasoningDelta { id, reasoning, .. }) => {
                self.reasoning_ids.insert(id);
                if let Some(clean) = self.reasoning.feed(&reasoning) {
                    if !clean.is_empty() {
                        out.push(StreamChunk::ReasoningDelta(clean));
                    }
                }
            }
            Ok(StreamedAssistantContent::Reasoning { reasoning, id }) => {
                // Complete block supersedes deltas on the same correlator —
                // emit only when no deltas streamed for it.
                if !self.reasoning_ids.contains(&id) {
                    let text = reasoning.display_text();
                    if !text.is_empty() {
                        out.push(StreamChunk::ReasoningDelta(text));
                    }
                }
            }
            Ok(StreamedAssistantContent::ToolCallDelta {
                internal_call_id,
                content,
            }) => {
                let slot = self.slot_for(&internal_call_id);
                match content {
                    ToolCallDeltaContent::Name(name) => {
                        self.slot_saw_deltas.insert(slot);
                        out.push(StreamChunk::ToolCallDelta {
                            slot,
                            id: None,
                            name: Some(name),
                            args_delta: String::new(),
                        });
                    }
                    ToolCallDeltaContent::Delta(args) => {
                        self.slot_saw_deltas.insert(slot);
                        out.push(StreamChunk::ToolCallDelta {
                            slot,
                            id: None,
                            name: None,
                            args_delta: args,
                        });
                    }
                }
            }
            Ok(StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            }) => {
                let slot = self.slot_for(&internal_call_id);
                let id = tool_call.id.to_string();
                out.push(StreamChunk::ToolCallDelta {
                    slot,
                    id: Some(id),
                    name: Some(tool_call.function.name),
                    args_delta: String::new(),
                });
                if !self.slot_saw_deltas.contains(&slot) {
                    // Whole-call wire (no fragments): feed the assembled
                    // arguments as one delta.
                    out.push(StreamChunk::ToolCallDelta {
                        slot,
                        id: None,
                        name: None,
                        args_delta: tool_call.function.arguments.to_string(),
                    });
                }
                // A structured call means any held `[call:` fragment was the
                // echo of this call — flush the stripper.
                if let Some(stripper) = self.call_stripper.as_mut() {
                    stripper.flush_call_line();
                }
            }
            Ok(StreamedAssistantContent::Final(f)) => {
                if !self.done {
                    self.done = true;
                    let u = f.usage;
                    out.push(StreamChunk::Done {
                        prompt_tokens: Some(u.input_tokens as u32),
                        completion_tokens: Some(u.output_tokens as u32),
                        cached_tokens: Some(
                            (u.cached_input_tokens + u.cache_creation_input_tokens) as u32
                        ),
                    });
                }
            }
            Ok(_) => {}
            Err(e) => {
                // Mid-stream decode failures on OpenAI wires are usually shim
                // framing leaks (non-JSON `data:` payloads) — the retired
                // adapters absorbed them; an abort here ended sessions, so
                // warn-and-continue.
                if self.swallow_decode_errors
                    && matches!(e, CompletionError::JsonError(_) | CompletionError::ResponseError(_))
                {
                    tracing::warn!(error = %e, "stream decode error swallowed");
                } else {
                    out.push(StreamChunk::Error(map_completion_error(&e)));
                }
            }
        }
        out
    }

    fn drain(&mut self) -> Vec<StreamChunk> {
        let mut out = Vec::new();
        if let Some(stripper) = self.call_stripper.as_mut() {
            if stripper.in_call {
                // Mid-`[call:` truncation — the held tail is call echo: try
                // to recover a call, then drop the residue.
                stripper.flush_call_line();
            } else if !stripper.hold.is_empty() {
                // Held tail is only a `[call:`-prefix candidate ("[", "[cal"
                // — e.g. the model ends on "参见下方 [附录]"): ordinary text
                // that must surface, not be dropped.
                let tail = std::mem::take(&mut stripper.hold);
                if let Some(clean) = self.text.feed(&tail) {
                    if !clean.is_empty() {
                        out.push(StreamChunk::ContentDelta(clean));
                    }
                }
            }
            for c in stripper.take_recovered() {
                let slot = self.next_slot;
                self.next_slot += 1;
                out.push(StreamChunk::ToolCallDelta {
                    slot,
                    id: Some(c.id),
                    name: Some(c.name),
                    args_delta: c.arguments,
                });
            }
        }
        if let Some(t) = self.text.flush() {
            out.push(StreamChunk::ContentDelta(t));
        }
        if let Some(r) = self.reasoning.flush() {
            out.push(StreamChunk::ReasoningDelta(r));
        }
        out
    }
}

/// rig `StreamingCompletionResponse` → our `StreamChunk` stream.
fn map_rig_stream(stream: StreamingCompletionResponse, kind: WireKind) -> BoxStream<StreamChunk> {
    struct S {
        inner: StreamingCompletionResponse,
        map: MapState,
        pending: std::collections::VecDeque<anyhow::Result<StreamChunk>>,
        drained: bool,
    }
    let init = S {
        inner: stream,
        map: MapState::new(kind),
        pending: std::collections::VecDeque::new(),
        drained: false,
    };
    Box::pin(futures::stream::unfold(init, |mut s| async move {
        loop {
            if let Some(c) = s.pending.pop_front() {
                return Some((c, s));
            }
            match s.inner.next().await {
                Some(item) => {
                    for c in s.map.map_one(item) {
                        s.pending.push_back(Ok(c));
                    }
                }
                None if !s.drained => {
                    s.drained = true;
                    for c in s.map.drain() {
                        s.pending.push_back(Ok(c));
                    }
                }
                None => return None,
            }
        }
    }))
}

// ================================================================
// CallStripper — `[call: name({json})]` text-protocol recovery
// ================================================================

/// Text-protocol escape hatch: some OpenAI-compatible backends echo tool
/// invocations as literal `[call: name({json})]` lines in the text stream.
/// Ported verbatim from the retired responses adapter.
struct CallStripper {
    /// Text held back because it might be a partial `[call:` prefix or
    /// inside an unterminated `[call: …` line.
    hold: String,
    /// True once we've entered a `[call:` line (until `]` or newline).
    in_call: bool,
    /// The accumulating `[call: …` payload — parsed into a ToolCall on `]`.
    call_buf: String,
    /// Tool calls recovered from `[call: name({json})]` text lines.
    recovered: Vec<crate::types::ToolCall>,
}

impl CallStripper {
    fn new() -> Self {
        Self {
            hold: String::new(),
            in_call: false,
            call_buf: String::new(),
            recovered: Vec::new(),
        }
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
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
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
    if b < 0x80 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
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

// ================================================================
// Error mapping
// ================================================================

/// `CompletionError` → the message shape `Sampler::classify_transport_error`
/// parses: `provider HTTP <code>:` when a status is known, otherwise the raw
/// error text (timeout/connect keywords stay visible to the retryable check).
fn map_completion_error(e: &CompletionError) -> String {
    match e {
        CompletionError::HttpError(http_client::Error::InvalidStatusCode(code)) => {
            format!("provider HTTP {code}: (empty body)")
        }
        CompletionError::HttpError(http_client::Error::InvalidStatusCodeWithMessage(
            code,
            body,
        )) => format!("provider HTTP {code}: {}", preview(body)),
        CompletionError::HttpError(http_client::Error::InvalidStatusCodeWithDetails {
            status,
            body,
            ..
        }) => format!("provider HTTP {status}: {}", preview(body)),
        CompletionError::HttpError(other) => format!("transport error: {other}"),
        CompletionError::ProviderResponse(p) => match p.status {
            Some(code) => format!("provider HTTP {code}: {}", preview(&p.body)),
            None => format!("provider error: {}", preview(&p.body)),
        },
        other => other.to_string(),
    }
}

/// Error bodies can be arbitrarily large — keep the first lines for the UI.
fn preview(s: &str) -> String {
    const MAX: usize = 2_000;
    if s.len() <= MAX {
        s.to_string()
    } else {
        format!("{}…", &s[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::completion::Usage;
    use rig_core::streaming::{StreamFinal, StreamFinalKind};

    fn chat(role: Role, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            is_error: None,
            notice: None,
            ts: None,
            images: Vec::new(),
            reasoning: None,
            duration_ms: None,
        }
    }

    fn rig_tool_call(id: &str, name: &str, args: Value) -> rig_core::message::ToolCall {
        rig_core::message::ToolCall {
            id: ToolCallId::new_or_mint(id),
            provider: None,
            function: ToolFunction { name: name.into(), arguments: args },
            signature: None,
            additional_params: None,
        }
    }

    fn rig_final(input: u64, output: u64) -> StreamedAssistantContent {
        StreamedAssistantContent::Final(StreamFinal {
            kind: StreamFinalKind::Final,
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input + output,
                ..Usage::default()
            },
            finish_reason: Some(rig_core::completion::FinishReason::Stop),
            message_id: None,
            response_id: None,
            provider_request_id: None,
            provider: "test".into(),
            model: None,
            raw: Value::Null,
        })
    }

    /// StreamedAssistantContent → StreamChunk: reasoning/text/call deltas,
    /// complete-call args-as-delta when no fragments streamed, usage → Done.
    #[test]
    fn map_state_maps_deltas_and_done() {
        let mut m = MapState::new(WireKind::Compat);
        let mut seen = Vec::new();
        let mut feed = |m: &mut MapState, item| {
            seen.extend(m.map_one(item));
        };
        feed(&mut m, Ok(StreamedAssistantContent::ReasoningDelta {
            id: "r".into(),
            provider_id: None,
            reasoning: "r1".into(),
        }));
        feed(&mut m, Ok(StreamedAssistantContent::Text(rig_core::message::Text::new("a".to_string()))));
        feed(&mut m, Ok(StreamedAssistantContent::ToolCallDelta {
            internal_call_id: "c".into(),
            content: ToolCallDeltaContent::Name("f".into()),
        }));
        feed(&mut m, Ok(StreamedAssistantContent::ToolCallDelta {
            internal_call_id: "c".into(),
            content: ToolCallDeltaContent::Delta("{".into()),
        }));
        // A complete ToolCall for the same slot: id+name emitted, args NOT
        // duplicated (deltas already carried the payload).
        feed(&mut m, Ok(StreamedAssistantContent::ToolCall {
            internal_call_id: "c".into(),
            tool_call: rig_tool_call("c", "f", json!({"x": 1})),
        }));
        feed(&mut m, Ok(rig_final(1, 2)));

        assert!(matches!(&seen[0], StreamChunk::ReasoningDelta(t) if t == "r1"));
        assert!(matches!(&seen[1], StreamChunk::ContentDelta(t) if t == "a"));
        assert!(matches!(&seen[2],
            StreamChunk::ToolCallDelta { slot: 0, name: Some(n), .. } if n == "f"));
        assert!(matches!(&seen[3],
            StreamChunk::ToolCallDelta { args_delta, .. } if args_delta == "{"));
        // complete call → id+name again (assembler dedups by slot)
        assert!(matches!(&seen[4],
            StreamChunk::ToolCallDelta { slot: 0, id: Some(i), name: Some(n), args_delta, .. }
            if i == "c" && n == "f" && args_delta.is_empty()));
        assert!(matches!(&seen[5], StreamChunk::Done {
            prompt_tokens: Some(1), completion_tokens: Some(2), .. }));
        assert_eq!(seen.len(), 6);
    }

    #[test]
    fn map_state_whole_call_emits_args_when_no_deltas() {
        let mut m = MapState::new(WireKind::Responses);
        let seen = m.map_one(Ok(StreamedAssistantContent::ToolCall {
            internal_call_id: "c1".into(),
            tool_call: rig_tool_call("c1", "f1", json!({"x": 1})),
        }));
        assert_eq!(seen.len(), 2);
        assert!(matches!(&seen[0],
            StreamChunk::ToolCallDelta { slot: 0, id: Some(i), name: Some(n), .. }
            if i == "c1" && n == "f1"));
        assert!(matches!(&seen[1],
            StreamChunk::ToolCallDelta { args_delta, .. } if args_delta.contains("\"x\"")));
    }

    /// Messages convert; tool pairing repair: orphan dropped, placeholder
    /// synthesized for an unresulted call.
    #[test]
    fn to_rig_messages_repairs_pairing() {
        let mut a1 = chat(Role::Assistant, "x");
        a1.tool_calls = Some(vec![crate::types::ToolCall {
            id: "c1".into(), name: "f".into(), arguments: "{}".into(),
        }]);
        let mut t1 = chat(Role::Tool, "r");
        t1.tool_call_id = Some("c1".into());
        let mut orphan = chat(Role::Tool, "orphan");
        orphan.tool_call_id = Some("nope".into());
        let mut a2 = chat(Role::Assistant, "");
        a2.tool_calls = Some(vec![crate::types::ToolCall {
            id: "c2".into(), name: "g".into(), arguments: "{}".into(),
        }]);

        let msgs = vec![
            chat(Role::System, "sys"),
            a1, t1, orphan, a2,
            chat(Role::User, "next"),
        ];
        let messages = to_rig_messages(&msgs, WireKind::Compat, &ProviderCompat::default());
        let serialized: Vec<Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap())
            .collect();
        // system + assistant(+call) + tool + assistant + user
        // (+placeholder result merged into the trailing user message)
        assert_eq!(serialized.len(), 5);
        assert!(serde_json::to_string(&serialized[4])
            .unwrap().contains("call interrupted"));
    }

    #[test]
    fn compat_patches_developer_role_and_stream_options() {
        let mut body = json!({
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [
                {"role": "system", "content": "s"},
                {"role": "assistant", "content": "a"},
            ],
        });
        let mut compat = ProviderCompat::default();
        compat.supports_developer_role = Some(true);
        compat_body_patches(&mut body, Some(&compat));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "developer");
        // stream_options removed when usage-in-streaming is unsupported
        assert!(body.get("stream_options").is_none());
    }

    #[test]
    fn compat_patches_reasoning_replay_and_tool_name() {
        let mut compat = ProviderCompat::default();
        compat.requires_reasoning_content_on_assistant_messages = Some(true);
        compat.requires_tool_result_name = Some(true);
        let mut body = json!({
            "reasoning_effort": "low",
            "messages": [
                {"role": "assistant", "content": "a",
                 "tool_calls": [{"id": "t1", "type": "function",
                    "function": {"name": "bash", "arguments": "{}"}}]},
                {"role": "tool", "tool_call_id": "t1", "content": "ok"},
            ],
        });
        compat_body_patches(&mut body, Some(&compat));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["reasoning_content"], "");
        assert_eq!(msgs[1]["name"], "bash");
    }

    /// Stream end with held `[call`-prefix text: the tail is ordinary model
    /// text — drain must emit it, not drop it (regression: flush used to
    /// clear `hold` unconditionally).
    #[test]
    fn drain_emits_held_call_prefix_as_text() {
        let mut m = MapState::new(WireKind::Compat);
        // Feed text that ends exactly on a `[call:`-prefix fragment.
        let mut seen = m.map_one(Ok(StreamedAssistantContent::Text(
            rig_core::message::Text::new("answer ends with [ca".to_string()),
        )));
        seen.extend(m.drain());
        assert!(
            seen.iter().any(|c| matches!(c, StreamChunk::ContentDelta(t) if t.contains("[ca"))),
            "held prefix text must surface on drain: {seen:?}"
        );
    }

    /// A structured ToolCall still drops held echo text — the structured
    /// event already carries the call; the `[call:` residue is the shim's
    /// textual echo of it.
    #[test]
    fn structured_call_drops_held_echo() {
        let mut m = MapState::new(WireKind::Compat);
        let mut seen = m.map_one(Ok(StreamedAssistantContent::Text(
            rig_core::message::Text::new(r#"calling [call: bash({"cmd":"ls"})]"#.to_string()),
        )));
        seen.extend(m.drain());
        // The `[call: bash(...)]` echo recovers as a real call — NOT text.
        assert!(
            seen.iter().any(|c| matches!(c, StreamChunk::ToolCallDelta { name: Some(n), .. } if n == "bash")),
            "{seen:?}"
        );
        assert!(
            !seen.iter().any(|c| matches!(c, StreamChunk::ContentDelta(t) if t.contains("[call:"))),
            "call echo must not leak as text: {seen:?}"
        );
    }

    /// `thinking.budget_tokens` clamps under a small max_tokens cap and
    /// disappears entirely when no legal budget fits.
    #[test]
    fn anthropic_budget_clamps_under_cap() {
        // max effort + 4k cap → clamped to cap-1.
        assert_eq!(anthropic_effective_budget(Some("max"), 4_096), Some(4_095));
        // cap too small for the 1024 minimum → thinking dropped.
        assert_eq!(anthropic_effective_budget(Some("low"), 1_024), None);
        // big cap → table value unchanged.
        assert_eq!(anthropic_effective_budget(Some("high"), 32_768), Some(16_384));
        // no effort → nothing.
        assert_eq!(anthropic_effective_budget(None, 32_768), None);
    }
}
