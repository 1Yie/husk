//! `config.toml` / `config.json` schema + secret indirection + hot-reload hooks.
//!
//! Supports both traditional TOML config and modern pi-agent / Devin compatible JSON formats.
//!
//! Resolution rules: `env:VAR` reads process env; `keyring:<service>/<account>`
//! reads the OS credential store; anything else is plaintext (warns at load).
//! A missing source marks the provider `available: false` — never panic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {0}: {1}")]
    Read(PathBuf, std::io::Error),

    #[error("failed to parse config file {0}: {1}")]
    Parse(PathBuf, String),
}

/// Top-level config document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(
        default,
        alias = "activeProvider",
        skip_serializing_if = "Option::is_none"
    )]
    pub active_provider: Option<String>,
    #[serde(
        default,
        alias = "activeModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub active_model: Option<String>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// Degrade chain: on unrecoverable 429/5xx, next provider takes over.
    #[serde(default, alias = "fallbackChain")]
    pub fallback_chain: Vec<String>,
}

/// User-level developer overrides — `~/.config/husk/settings.toml`.
///
/// **Why a dedicated file:** these settings can prevent the GUI from
/// starting (`renderer = "wayland"` on a driver that can't do it,
/// `gpu_acceleration = false` on a stack that needs hardware GL). Keeping
/// them out of `config.toml` (which holds providers/keys) means a bad
/// value lives beside the file a user is already expected to edit by
/// hand — and the recovery path is a single obvious file, not a buried
/// section of the provider config.
///
/// All fields `Option`: `None` = "not set — follow the compiled default".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DevConfig {
    /// Rendering backend for GTK/WebKitGTK: `"x11"` (default, XWayland-safe) |
    /// `"wayland"` (native — fixes some HiDPI/fractional-scale paths, breaks
    /// on drivers without a working Wayland GL stack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<String>,
    /// `false` → software GL: `WEBKIT_DISABLE_DMABUF_RENDERER=1` and
    /// `LIBGL_ALWAYS_SOFTWARE=1`. `true` → opt into the dmabuf GPU
    /// renderer. `None` → compiled NVIDIA-safe default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_acceleration: Option<bool>,
    /// `true` → the Shift+Ctrl+P monitor overlay is enabled (the window's
    /// keyboard shortcut hook is wired only when this is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_panel: Option<bool>,
    /// `true` → F12 / Ctrl+Shift+I/C/J/K shortcuts reach the WebKitGTK
    /// inspector in a packaged build (dev mode keeps them anyway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devtools: Option<bool>,
    /// `true` → the "开发者选项" settings entry is visible in the nav.
    /// Hidden by default — a typical user never touches renderer flags,
    /// and the only way to surface this pane is hand-editing this file
    /// (`developer_ui = true`) and relaunching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_ui: Option<bool>,
}

impl DevConfig {
    /// The canonical user-level path: `~/.config/husk/settings.toml` —
    /// same directory as `config.toml`, separate file so a bad renderer
    /// flag never sits next to provider secrets.
    pub fn default_path() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        Some(base.join("husk").join("settings.toml"))
    }

    /// Load from `~/.config/husk/settings.toml`; missing file → default.
    pub fn load() -> Result<Self, ConfigError> {
        let Some(path) = Self::default_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| ConfigError::Read(path.clone(), e))?;
        toml::from_str(&raw).map_err(|e| ConfigError::Parse(path.clone(), e.to_string()))
    }

    /// Write `~/.config/husk/settings.toml` — the whole file is ours, so
    /// this overwrites rather than merges.
    pub fn save(dev: &DevConfig) -> Result<(), ConfigError> {
        let Some(path) = Self::default_path() else {
            return Err(ConfigError::Read(
                PathBuf::from("~/.config/husk/settings.toml"),
                std::io::Error::new(std::io::ErrorKind::NotFound, "config dir unavailable"),
            ));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Read(parent.to_path_buf(), e))?;
        }
        let out = toml::to_string_pretty(dev)
            .map_err(|e| ConfigError::Parse(path.clone(), e.to_string()))?;
        std::fs::write(&path, out).map_err(|e| ConfigError::Read(path.clone(), e))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(alias = "type", alias = "api")]
    pub kind: ProviderKind,
    #[serde(alias = "base_url", alias = "baseUrl")]
    pub base_url: String,
    /// `env:VAR` | `keyring:<service>/<account>` | plaintext (warns).
    #[serde(default, alias = "api_key", alias = "apiKey")]
    pub api_key: String,
    /// Optional static headers (e.g. `{"HTTP-Referer" = "…"}`).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Defaults per-provider (rare) — model usually comes from `active_model`.
    #[serde(
        default,
        alias = "default_model",
        alias = "defaultModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_model: Option<String>,
    /// Compatibility flags (matching pi-agent / Devin format).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<ProviderCompat>,
    /// Configured models for this provider (can be simple string IDs or rich objects).
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Display name for the provider (pi `providers.<id>.name`) — the map key
    /// stays the identifier; this is the human label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `true` adds `Authorization: Bearer <apiKey>` to every request
    /// (pi `authHeader`). `None` keeps each adapter's own default.
    #[serde(
        default,
        alias = "auth_header",
        alias = "authHeader",
        skip_serializing_if = "Option::is_none"
    )]
    pub auth_header: Option<bool>,
    /// Per-model overrides merged onto `models` by id at load time
    /// (pi `modelOverrides`). Unknown ids are ignored.
    #[serde(
        default,
        alias = "model_overrides",
        alias = "modelOverrides",
        skip_serializing_if = "Option::is_none"
    )]
    pub model_overrides: Option<HashMap<String, ModelOverride>>,
}

impl ProviderConfig {
    /// Return all model IDs configured for this provider.
    pub fn model_ids(&self) -> Vec<String> {
        self.models.iter().map(|m| m.id().to_string()).collect()
    }

    /// Find model entry by ID.
    pub fn find_model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id() == id)
    }

    /// The provider's own opts, with `modelOverrides` for `model_id` applied.
    /// A model with no override entry returns the config unchanged.
    pub fn model_opts(&self, model_id: &str) -> ModelConfig {
        let base = self
            .find_model(model_id)
            .and_then(|m| m.detailed().cloned())
            .unwrap_or_else(|| ModelConfig {
                id: model_id.to_string(),
                ..Default::default()
            });
        match self.model_overrides.as_ref().and_then(|o| o.get(model_id)) {
            None => base,
            Some(ov) => base.with_override(ov),
        }
    }

    /// The provider-level `compat` with any model-level `compat` merged over it
    /// (pi: model wins on every field it declares).
    pub fn effective_compat(&self, model_id: &str) -> ProviderCompat {
        let provider = self.compat.clone().unwrap_or_default();
        let model = self
            .find_model(model_id)
            .and_then(|m| m.detailed())
            .and_then(|d| d.compat.as_ref())
            .or_else(|| {
                self.model_overrides
                    .as_ref()?
                    .get(model_id)?
                    .compat
                    .as_ref()
            });
        provider.merged_with(model)
    }

    /// Provider headers with the model's own merged over them.
    pub fn effective_headers(&self, model_id: &str) -> HashMap<String, String> {
        let mut out = self.headers.clone();
        let model = self
            .find_model(model_id)
            .and_then(|m| m.detailed())
            .and_then(|d| d.headers.as_ref())
            .or_else(|| {
                self.model_overrides
                    .as_ref()?
                    .get(model_id)?
                    .headers
                    .as_ref()
            });
        if let Some(extra) = model {
            for (k, v) in extra {
                out.insert(k.clone(), v.clone());
            }
        }
        out
    }

    /// Non-fatal problems in this provider's models — reported as load-time
    /// warnings, never as a hard error.
    pub fn validate(&self, provider_name: &str) -> Vec<ConfigWarning> {
        let mut out = Vec::new();
        let mut names: Vec<(String, &ModelConfig)> = Vec::new();
        for m in &self.models {
            if let Some(d) = m.detailed() {
                names.push((d.id.clone(), d));
            }
        }
        for (id, m) in names {
            if let (Some(max), Some(ctx)) = (m.max_tokens, m.context_window) {
                if max > ctx {
                    out.push(ConfigWarning::MaxTokensOverContext {
                        provider: provider_name.to_string(),
                        model: id.clone(),
                        max_tokens: max,
                        context_window: ctx,
                    });
                }
            }
            if let Some(map) = &m.thinking_level_map {
                for level in map.keys() {
                    if !THINKING_LEVELS.contains(&level.as_str()) {
                        out.push(ConfigWarning::UnknownThinkingLevel {
                            provider: provider_name.to_string(),
                            model: id.clone(),
                            level: level.clone(),
                        });
                    }
                }
            }
        }
        out
    }
}

/// Feature compatibility flags — the full pi (`models.json`) `compat` surface.
///
/// Every field is optional; `None` means "not declared" and the adapter's default
/// applies. Set per provider or per model, the model winning. Rust snake_case names
/// accept the pi camelCase spelling as an alias, so `models.json` and a
/// settings-written `config.toml` both round-trip.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderCompat {
    // ---- OpenAI-compatible (openai-completions / openai-responses) ----
    /// Provider accepts the `store` request field.
    #[serde(
        default,
        alias = "supports_store",
        alias = "supportsStore",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_store: Option<bool>,
    /// Send the system prompt as `developer` (true) or `system` (false).
    #[serde(
        default,
        alias = "supports_developer_role",
        alias = "supportsDeveloperRole",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_developer_role: Option<bool>,
    /// Provider accepts `reasoning_effort`. `false` is the per-deployment kill
    /// switch — the engine then never computes an effort value at all.
    #[serde(
        default,
        alias = "supports_reasoning_effort",
        alias = "supportsReasoningEffort",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_reasoning_effort: Option<bool>,
    /// Output-token cap field: `max_completion_tokens` or `max_tokens`.
    /// `None` → the adapter's own default.
    #[serde(
        default,
        alias = "max_tokens_field",
        alias = "maxTokensField",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens_field: Option<String>,
    /// Provider accepts `stream_options: { include_usage: true }` (pi default true).
    #[serde(
        default,
        alias = "supports_usage_in_streaming",
        alias = "supportsUsageInStreaming",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_usage_in_streaming: Option<bool>,
    /// Streamed responses carry `finish_reason`; when false the adapter infers
    /// `stop` / `toolUse` from stream termination (pi default true).
    #[serde(
        default,
        alias = "supports_finish_reason",
        alias = "supportsFinishReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_finish_reason: Option<bool>,
    /// Include `name` on tool-result messages.
    #[serde(
        default,
        alias = "requires_tool_result_name",
        alias = "requiresToolResultName",
        skip_serializing_if = "Option::is_none"
    )]
    pub requires_tool_result_name: Option<bool>,
    /// Insert an assistant message between tool results and a following user message.
    #[serde(
        default,
        alias = "requires_assistant_after_tool_result",
        alias = "requiresAssistantAfterToolResult",
        skip_serializing_if = "Option::is_none"
    )]
    pub requires_assistant_after_tool_result: Option<bool>,
    /// Convert thinking blocks to plain text instead of provider-native reasoning.
    #[serde(
        default,
        alias = "requires_thinking_as_text",
        alias = "requiresThinkingAsText",
        skip_serializing_if = "Option::is_none"
    )]
    pub requires_thinking_as_text: Option<bool>,
    /// Replay an empty `reasoning_content` on every assistant message while
    /// reasoning is enabled (some backends require the key to be present).
    #[serde(
        default,
        alias = "requires_reasoning_content_on_assistant_messages",
        alias = "requiresReasoningContentOnAssistantMessages",
        skip_serializing_if = "Option::is_none"
    )]
    pub requires_reasoning_content_on_assistant_messages: Option<bool>,
    /// Thinking-parameter dialect: `openai`, `openrouter`, `deepseek`,
    /// `together`, `baseten`, `zai`, `qwen`, `chat-template`,
    /// `qwen-chat-template`, `string-thinking`, `ant-ling`.
    #[serde(
        default,
        alias = "thinking_format",
        alias = "thinkingFormat",
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_format: Option<String>,
    /// `chat_template_kwargs` values for `thinkingFormat: "chat-template"`.
    #[serde(
        default,
        alias = "chat_template_kwargs",
        alias = "chatTemplateKwargs",
        skip_serializing_if = "Option::is_none"
    )]
    pub chat_template_kwargs: Option<serde_json::Value>,
    /// `chat_template_args` values for `thinkingFormat: "baseten"`.
    #[serde(
        default,
        alias = "chat_template_args",
        alias = "chatTemplateArgs",
        skip_serializing_if = "Option::is_none"
    )]
    pub chat_template_args: Option<serde_json::Value>,
    /// Top-level request field capping reasoning tokens: `thinking_token_budget`
    /// (vLLM), `thinking_budget` (Qwen/DashScope/SGLang),
    /// `thinking_budget_tokens` (llama.cpp).
    #[serde(
        default,
        alias = "thinking_token_budget_field",
        alias = "thinkingTokenBudgetField",
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_token_budget_field: Option<String>,
    /// Alias for `thinking_token_budget_field` — `true` means the default field.
    #[serde(
        default,
        alias = "supports_thinking_token_budget",
        alias = "supportsThinkingTokenBudget",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_thinking_token_budget: Option<bool>,
    /// Anthropic-style `cache_control` markers on an OpenAI-compatible wire.
    /// Only `"anthropic"` is meaningful.
    #[serde(
        default,
        alias = "cache_control_format",
        alias = "cacheControlFormat",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_control_format: Option<String>,
    /// Send session-affinity headers derived from the session id.
    #[serde(
        default,
        alias = "send_session_affinity_headers",
        alias = "sendSessionAffinityHeaders",
        skip_serializing_if = "Option::is_none"
    )]
    pub send_session_affinity_headers: Option<bool>,
    /// Session-affinity header dialect: `openai`, `openai-nosession`, `openrouter`.
    #[serde(
        default,
        alias = "session_affinity_format",
        alias = "sessionAffinityFormat",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_affinity_format: Option<String>,
    /// Provider accepts strict JSON-schema function tool definitions.
    #[serde(
        default,
        alias = "supports_strict_mode",
        alias = "supportsStrictMode",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_strict_mode: Option<bool>,
    /// Emit custom Lark/regex grammar tools (GPT-5+ on OpenAI and friends).
    #[serde(
        default,
        alias = "supports_openai_grammar_tools",
        alias = "supportsOpenAIGrammarTools",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_openai_grammar_tools: Option<bool>,
    /// Accept long cache retention when cache retention is `long`.
    #[serde(
        default,
        alias = "supports_long_cache_retention",
        alias = "supportsLongCacheRetention",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_long_cache_retention: Option<bool>,
    /// OpenRouter provider-routing preferences — sent verbatim as the request
    /// `provider` field.
    #[serde(
        default,
        alias = "open_router_routing",
        alias = "openRouterRouting",
        skip_serializing_if = "Option::is_none"
    )]
    pub open_router_routing: Option<serde_json::Value>,
    /// Vercel AI Gateway routing config (`only`, `order`).
    #[serde(
        default,
        alias = "vercel_gateway_routing",
        alias = "vercelGatewayRouting",
        skip_serializing_if = "Option::is_none"
    )]
    pub vercel_gateway_routing: Option<serde_json::Value>,

    // ---- Anthropic Messages (anthropic-messages) ----
    /// Per-tool `eager_input_streaming: true` (pi default true). `false` omits
    /// the field and sends the legacy fine-grained-tool-streaming beta header.
    #[serde(
        default,
        alias = "supports_eager_tool_input_streaming",
        alias = "supportsEagerToolInputStreaming",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_eager_tool_input_streaming: Option<bool>,
    /// `cache_control.ttl: "1h"` on tool definitions is accepted.
    #[serde(
        default,
        alias = "supports_cache_control_on_tools",
        alias = "supportsCacheControlOnTools",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_cache_control_on_tools: Option<bool>,
    /// Send adaptive thinking (`thinking.type: "adaptive"` + `output_config.effort`).
    #[serde(
        default,
        alias = "force_adaptive_thinking",
        alias = "forceAdaptiveThinking",
        skip_serializing_if = "Option::is_none"
    )]
    pub force_adaptive_thinking: Option<bool>,
    /// Per-turn effort system messages + thinking-binding controls
    /// (`prefix_mismatch_behavior: "drop_block"`).
    #[serde(
        default,
        alias = "supports_mid_convo_effort",
        alias = "supportsMidConvoEffort",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_mid_convo_effort: Option<bool>,
    /// Replay empty thinking signatures as `signature: ""` instead of turning
    /// thinking into text — only for providers that emit empty signatures.
    #[serde(
        default,
        alias = "allow_empty_signature",
        alias = "allowEmptySignature",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_empty_signature: Option<bool>,
    /// Provider accepts strict JSON-schema tool definitions (Anthropic flavour).
    #[serde(
        default,
        alias = "supports_strict_tools",
        alias = "supportsStrictTools",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_strict_tools: Option<bool>,
    /// Up to three server-side fallback models, each `{provider, model, cost}`.
    /// An empty array disables fallback.
    #[serde(
        default,
        alias = "allowed_fallback_models",
        alias = "allowedFallbackModels",
        skip_serializing_if = "Option::is_none"
    )]
    pub allowed_fallback_models: Option<serde_json::Value>,
}

/// Every optional field of `ProviderCompat`, for field-wise merge — a macro
/// keeps the list in one place so a new field is never silently un-merged.
macro_rules! compat_fields {
    ($mac:ident) => {
        $mac! {
            supports_store,
            supports_developer_role,
            supports_reasoning_effort,
            max_tokens_field,
            supports_usage_in_streaming,
            supports_finish_reason,
            requires_tool_result_name,
            requires_assistant_after_tool_result,
            requires_thinking_as_text,
            requires_reasoning_content_on_assistant_messages,
            thinking_format,
            chat_template_kwargs,
            chat_template_args,
            thinking_token_budget_field,
            supports_thinking_token_budget,
            cache_control_format,
            send_session_affinity_headers,
            session_affinity_format,
            supports_strict_mode,
            supports_openai_grammar_tools,
            supports_long_cache_retention,
            open_router_routing,
            vercel_gateway_routing,
            supports_eager_tool_input_streaming,
            supports_cache_control_on_tools,
            force_adaptive_thinking,
            supports_mid_convo_effort,
            allow_empty_signature,
            supports_strict_tools,
            allowed_fallback_models,
        }
    };
}

impl ProviderCompat {
    /// Field-wise merge: every field `over` declares replaces `self`'s; fields
    /// it leaves `None` keep the base value. This is pi's semantic for
    /// provider `compat` → model `compat`.
    pub fn merged_with(&self, over: Option<&ProviderCompat>) -> ProviderCompat {
        let Some(over) = over else {
            return self.clone();
        };
        let mut out = self.clone();
        macro_rules! take {
            ($($f:ident),* $(,)?) => {
                $(if over.$f.is_some() { out.$f = over.$f.clone(); })*
            };
        }
        compat_fields!(take);
        out
    }

    /// Whether the system prompt should be sent with the `developer` role.
    /// OpenAI's own default is `developer` for reasoning models; a proxy that
    /// only knows `system` sets `supportsDeveloperRole: false` (pi semantics).
    pub fn developer_role(&self) -> bool {
        self.supports_developer_role.unwrap_or(false)
    }

    /// The output-token cap field name — `max_completion_tokens` unless the
    /// provider declares `max_tokens` (pi's `maxTokensField`).
    pub fn max_tokens_field_name(&self) -> &str {
        self.max_tokens_field
            .as_deref()
            .unwrap_or("max_completion_tokens")
    }

    /// The reasoning-token budget field, resolving the
    /// `supportsThinkingTokenBudget` alias (`true` → the default field name).
    pub fn thinking_token_budget_field_name(&self) -> Option<&str> {
        if let Some(f) = self.thinking_token_budget_field.as_deref() {
            return Some(f);
        }
        self.supports_thinking_token_budget
            .filter(|enabled| *enabled)
            .map(|_| "thinking_token_budget")
    }

    /// The effective thinking dialect (`openai` when undeclared).
    pub fn thinking_format_name(&self) -> &str {
        self.thinking_format.as_deref().unwrap_or("openai")
    }
}

/// A model entry can be either a bare string ID or a rich pi-agent model object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ModelEntry {
    Simple(String),
    Detailed(ModelConfig),
}

impl ModelEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Detailed(m) => m.id.as_str(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Simple(s) => s.as_str(),
            Self::Detailed(m) => m.name.as_deref().unwrap_or(m.id.as_str()),
        }
    }

    pub fn detailed(&self) -> Option<&ModelConfig> {
        match self {
            Self::Detailed(m) => Some(m),
            Self::Simple(_) => None,
        }
    }
}

impl From<String> for ModelEntry {
    fn from(s: String) -> Self {
        Self::Simple(s)
    }
}

impl From<&str> for ModelEntry {
    fn from(s: &str) -> Self {
        Self::Simple(s.to_string())
    }
}

impl From<ModelConfig> for ModelEntry {
    fn from(m: ModelConfig) -> Self {
        Self::Detailed(m)
    }
}

/// Rich model definition matching the pi `models.json` model schema.
///
/// Every field beyond `id` is optional — a bare `{ "id": "…" }` entry is
/// valid and takes every default. Unknown keys are ignored by serde, so a
/// pi `models.json` pasted into `config.json` loads without edits.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// API type override for this specific model (pi model-level `api`) —
    /// falls back to the provider's `kind` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ProviderKind>,
    /// API endpoint override for this specific model (pi model-level `baseUrl`).
    #[serde(
        default,
        alias = "base_url",
        alias = "baseUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    /// Supported input modalities — `["text"]` (default) or `["text", "image"]`.
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(
        default,
        alias = "context_window",
        alias = "contextWindow",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window: Option<u64>,
    /// Maximum output tokens (pi `maxTokens`). `None` → the adapter's own
    /// default cap.
    #[serde(
        default,
        alias = "max_output_tokens",
        alias = "maxTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
    #[serde(
        default,
        alias = "thinking_level_map",
        alias = "thinkingLevelMap",
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_level_map: Option<HashMap<String, Option<String>>>,
    /// Sampling parameters merged verbatim into every request body — after the
    /// fields the adapter sets itself, so these keys win. For OpenAI-compatible
    /// APIs only.
    #[serde(
        default,
        alias = "sampling_params",
        alias = "samplingParams",
        skip_serializing_if = "Option::is_none"
    )]
    pub sampling_params: Option<serde_json::Value>,
    /// Request limits / image preprocessing (pi `inputLimits`).
    #[serde(
        default,
        alias = "input_limits",
        alias = "inputLimits",
        skip_serializing_if = "Option::is_none"
    )]
    pub input_limits: Option<InputLimits>,
    /// Best-effort prompt-cache lifetime in seconds per retention tier
    /// (pi `promptCache`). Unset disables cache warming for the model.
    #[serde(
        default,
        alias = "prompt_cache",
        alias = "promptCache",
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt_cache: Option<PromptCache>,
    /// Per-model compatibility overrides — merged over the provider's `compat`
    /// (model wins on every field it declares).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<ProviderCompat>,
    /// Custom headers for this specific model, merged over the provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

/// Non-fatal model issues found while validating a config document — surfaced
/// as load-time warnings instead of hard errors (a hand-written config with
/// one bad model must not take the whole provider offline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWarning {
    /// `maxTokens` exceeds the model's `contextWindow` — the request could
    /// never fit its own output budget.
    MaxTokensOverContext {
        provider: String,
        model: String,
        max_tokens: u64,
        context_window: u64,
    },
    /// `thinkingLevelMap` key is not a pi thinking level — ignored.
    UnknownThinkingLevel {
        provider: String,
        model: String,
        level: String,
    },
}

impl std::fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MaxTokensOverContext { provider, model, max_tokens, context_window } => write!(
                f,
                "{provider}/{model}: maxTokens ({max_tokens}) exceeds contextWindow ({context_window})"
            ),
            Self::UnknownThinkingLevel { provider, model, level } => write!(
                f,
                "{provider}/{model}: unknown thinking level `{level}` in thinkingLevelMap"
            ),
        }
    }
}

/// The pi thinking levels, in UI order — the only valid `thinkingLevelMap` keys.
pub const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Request limits and image preprocessing for one model (pi `inputLimits`).
/// The hard limits are descriptive metadata; only `images.resize` is applied.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InputLimits {
    /// Hard provider cap on a single request body, in bytes.
    #[serde(
        default,
        alias = "max_request_bytes",
        alias = "maxRequestBytes",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_request_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<ImageLimits>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImageLimits {
    /// Maximum images in one message.
    #[serde(
        default,
        alias = "max_per_message",
        alias = "maxPerMessage",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_per_message: Option<u32>,
    /// Maximum images in one request.
    #[serde(
        default,
        alias = "max_per_request",
        alias = "maxPerRequest",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_per_request: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resize: Option<ImageResize>,
}

/// How images are re-encoded before entering conversation history. Omitted
/// fields fall back to pi's conservative defaults: 2000×2000, 4.5 MiB encoded,
/// JPEG quality 80 — see [`ImageResize::resolved`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImageResize {
    #[serde(
        default,
        alias = "max_width",
        alias = "maxWidth",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_width: Option<u32>,
    #[serde(
        default,
        alias = "max_height",
        alias = "maxHeight",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_height: Option<u32>,
    /// Maximum base64-encoded payload size, in bytes.
    #[serde(
        default,
        alias = "max_bytes",
        alias = "maxBytes",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_bytes: Option<u64>,
    #[serde(
        default,
        alias = "jpeg_quality",
        alias = "jpegQuality",
        skip_serializing_if = "Option::is_none"
    )]
    pub jpeg_quality: Option<u8>,
}

impl ImageResize {
    /// pi's documented defaults for omitted fields.
    pub const DEFAULT_MAX_WIDTH: u32 = 2000;
    pub const DEFAULT_MAX_HEIGHT: u32 = 2000;
    pub const DEFAULT_MAX_BYTES: u64 = 4_718_592; // 4.5 MiB encoded
    pub const DEFAULT_JPEG_QUALITY: u8 = 80;

    /// The effective profile — declared values, pi defaults elsewhere.
    pub fn resolved(&self) -> (u32, u32, u64, u8) {
        (
            self.max_width.unwrap_or(Self::DEFAULT_MAX_WIDTH),
            self.max_height.unwrap_or(Self::DEFAULT_MAX_HEIGHT),
            self.max_bytes.unwrap_or(Self::DEFAULT_MAX_BYTES),
            self.jpeg_quality.unwrap_or(Self::DEFAULT_JPEG_QUALITY),
        )
    }
}

/// Best-effort prompt-cache lifetime, in seconds, per retention tier
/// (pi `promptCache`). `short` is the tier a normal request uses; `long` is
/// used when long cache retention is requested.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PromptCache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long: Option<u64>,
}

impl PromptCache {
    /// Lifetime for the tier a request used — `None` means "not warmed".
    pub fn lifetime(&self, long: bool) -> Option<u64> {
        if long {
            self.long
        } else {
            self.short
        }
    }
}

/// `modelOverrides` entry — every field optional, merged onto the matching
/// built-in/configured model by id. Mirrors the fields pi documents as
/// overridable; absent fields keep the base model's value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(
        default,
        alias = "thinking_level_map",
        alias = "thinkingLevelMap",
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_level_map: Option<HashMap<String, Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    #[serde(
        default,
        alias = "input_limits",
        alias = "inputLimits",
        skip_serializing_if = "Option::is_none"
    )]
    pub input_limits: Option<InputLimits>,
    /// Partial cost — declared keys overwrite, absent keys keep the base rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
    #[serde(
        default,
        alias = "prompt_cache",
        alias = "promptCache",
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt_cache: Option<PromptCache>,
    #[serde(
        default,
        alias = "context_window",
        alias = "contextWindow",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window: Option<u64>,
    #[serde(
        default,
        alias = "max_output_tokens",
        alias = "maxTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tokens: Option<u64>,
    #[serde(
        default,
        alias = "sampling_params",
        alias = "samplingParams",
        skip_serializing_if = "Option::is_none"
    )]
    pub sampling_params: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<ProviderCompat>,
}

impl ModelConfig {
    /// Apply a `modelOverrides` entry — declared fields win, absent fields keep
    /// this model's value. Maps (`cost`, `promptCache`, `samplingParams`,
    /// `headers`, `inputLimits`, `compat`) merge per key rather than replacing
    /// wholesale, matching pi's override semantics.
    pub fn with_override(&self, ov: &ModelOverride) -> ModelConfig {
        let mut out = self.clone();
        if let Some(v) = &ov.name {
            out.name = Some(v.clone());
        }
        if let Some(v) = ov.reasoning {
            out.reasoning = Some(v);
        }
        if let Some(v) = &ov.thinking_level_map {
            let mut merged = out.thinking_level_map.take().unwrap_or_default();
            for (k, v) in v {
                merged.insert(k.clone(), v.clone());
            }
            out.thinking_level_map = Some(merged);
        }
        if let Some(v) = &ov.input {
            out.input = v.clone();
        }
        if let Some(v) = &ov.input_limits {
            out.input_limits = Some(match out.input_limits.take() {
                None => v.clone(),
                Some(base) => base.merged_with(v),
            });
        }
        if let Some(v) = &ov.cost {
            out.cost = Some(match out.cost.take() {
                None => v.clone(),
                Some(base) => base.merged_with(v),
            });
        }
        if let Some(v) = &ov.prompt_cache {
            out.prompt_cache = Some(match out.prompt_cache.take() {
                None => v.clone(),
                Some(base) => PromptCache {
                    short: v.short.or(base.short),
                    long: v.long.or(base.long),
                },
            });
        }
        if let Some(v) = ov.context_window {
            out.context_window = Some(v);
        }
        if let Some(v) = ov.max_tokens {
            out.max_tokens = Some(v);
        }
        // `samplingParams` merges per key (pi) — shallow object merge.
        if let Some(v) = &ov.sampling_params {
            out.sampling_params = Some(match out.sampling_params.take() {
                Some(serde_json::Value::Object(mut base)) => {
                    if let Some(extra) = v.as_object() {
                        for (k, val) in extra {
                            base.insert(k.clone(), val.clone());
                        }
                    }
                    serde_json::Value::Object(base)
                }
                _ => v.clone(),
            });
        }
        if let Some(v) = &ov.headers {
            let mut merged = out.headers.take().unwrap_or_default();
            for (k, val) in v {
                merged.insert(k.clone(), val.clone());
            }
            out.headers = Some(merged);
        }
        if let Some(v) = &ov.compat {
            out.compat = Some(out.compat.take().unwrap_or_default().merged_with(Some(v)));
        }
        out
    }

    /// The sampling-parameter object to merge into the request body, if any.
    pub fn sampling_object(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.sampling_params.as_ref().and_then(|v| v.as_object())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(
        default,
        alias = "cache_read",
        alias = "cacheRead",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_read: Option<f64>,
    #[serde(
        default,
        alias = "cache_write",
        alias = "cacheWrite",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_write: Option<f64>,
    /// Request-wide input pricing tiers (pi `cost.tiers`). A tier applies to
    /// the whole request once total input usage (`input + cacheRead +
    /// cacheWrite`) exceeds its `input_tokens_above`; when several match, the
    /// highest threshold wins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<CostTier>,
}

/// One request-wide pricing tier — a complete alternate rate set.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CostTier {
    /// Total input tokens (input + cacheRead + cacheWrite) above which this
    /// tier's rates apply to the entire request.
    pub input_tokens_above: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(
        default,
        alias = "cache_read",
        alias = "cacheRead",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_read: Option<f64>,
    #[serde(
        default,
        alias = "cache_write",
        alias = "cacheWrite",
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_write: Option<f64>,
}

impl ModelCost {
    /// The rate set in effect for a request that consumed `total_input`
    /// tokens of input (prompt + cache read + cache write). Base rates when no
    /// tier matches, else the matching tier with the highest threshold —
    /// absent tier rates fall back to the base rate for that line item.
    pub fn effective(&self, total_input: u64) -> ModelCost {
        let best = self
            .tiers
            .iter()
            .filter(|t| total_input > t.input_tokens_above)
            .max_by_key(|t| t.input_tokens_above);
        match best {
            None => self.clone(),
            Some(t) => ModelCost {
                input: t.input.or(self.input),
                output: t.output.or(self.output),
                cache_read: t.cache_read.or(self.cache_read),
                cache_write: t.cache_write.or(self.cache_write),
                // Tiers are not nested — the effective set carries no further tiers.
                tiers: Vec::new(),
            },
        }
    }

    /// Cost in USD for one usage tuple at `total_input`-selected rates.
    pub fn price(
        &self,
        total_input: u64,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> f64 {
        let r = self.effective(total_input);
        let per_m =
            |tokens: u64, rate: Option<f64>| tokens as f64 * rate.unwrap_or(0.0) / 1_000_000.0;
        per_m(input, r.input)
            + per_m(output, r.output)
            + per_m(cache_read, r.cache_read)
            + per_m(cache_write, r.cache_write)
    }

    /// Key-wise merge for `modelOverrides.cost`: declared keys overwrite,
    /// absent keys keep the base rate. `tiers` replaces wholesale when the
    /// override declares any.
    pub fn merged_with(&self, over: &ModelCost) -> ModelCost {
        ModelCost {
            input: over.input.or(self.input),
            output: over.output.or(self.output),
            cache_read: over.cache_read.or(self.cache_read),
            cache_write: over.cache_write.or(self.cache_write),
            tiers: if over.tiers.is_empty() {
                self.tiers.clone()
            } else {
                over.tiers.clone()
            },
        }
    }
}

impl InputLimits {
    /// Deep merge for `modelOverrides.inputLimits` — nested `images.resize`
    /// merges per field.
    pub fn merged_with(&self, over: &InputLimits) -> InputLimits {
        InputLimits {
            max_request_bytes: over.max_request_bytes.or(self.max_request_bytes),
            images: match (&self.images, &over.images) {
                (None, None) => None,
                (Some(base), None) => Some(base.clone()),
                (None, Some(o)) => Some(o.clone()),
                (Some(base), Some(o)) => Some(ImageLimits {
                    max_per_message: o.max_per_message.or(base.max_per_message),
                    max_per_request: o.max_per_request.or(base.max_per_request),
                    resize: match (&base.resize, &o.resize) {
                        (None, None) => None,
                        (Some(b), None) => Some(b.clone()),
                        (None, Some(r)) => Some(r.clone()),
                        (Some(b), Some(r)) => Some(ImageResize {
                            max_width: r.max_width.or(b.max_width),
                            max_height: r.max_height.or(b.max_height),
                            max_bytes: r.max_bytes.or(b.max_bytes),
                            jpeg_quality: r.jpeg_quality.or(b.jpeg_quality),
                        }),
                    },
                }),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProviderKind {
    #[serde(
        rename = "openai_compat",
        alias = "openai-compat",
        alias = "openai_completions",
        alias = "openai-completions"
    )]
    #[default]
    OpenaiCompat,
    #[serde(rename = "openai_responses", alias = "openai-responses")]
    OpenaiResponses,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
}

/// Outcome of resolving one provider's secret indirection.
#[derive(Debug)]
pub enum SecretResolution {
    /// Raw secret material, already resolved — feed to the adapter.
    Resolved(String),
    /// Source declared but unresolvable (env unset, keyring miss).
    /// Provider must be marked unavailable, never panic.
    Unavailable(String),
    /// Plaintext in the file — usable but flagged for a load-time warning.
    Plaintext(String),
    /// No key configured at all (e.g. Ollama doesn't need one).
    Empty,
}

impl AppConfig {
    /// Default config path: `~/.config/husk/config.json` (if exists) or `config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        let dir = base.join("husk");
        // Renamed from `agent-rs` — a one-time `fs::rename` carries config +
        // plugins over when the new dir is absent.
        let legacy = base.join("agent-rs");
        if !dir.exists() && legacy.is_dir() {
            let _ = std::fs::rename(&legacy, &dir);
        }
        // TOML is canonical — a hand-edited config.toml is ALWAYS the
        // live file when it exists; config.json only fills in otherwise.
        let toml_path = dir.join("config.toml");
        if toml_path.exists() {
            return Some(toml_path);
        }
        let json_path = dir.join("config.json");
        if json_path.exists() {
            return Some(json_path);
        }
        Some(toml_path)
    }

    /// Load from disk; missing file yields `AppConfig::default()` (first-run).
    /// Supports both JSON and TOML syntax, and naked provider mappings.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => match Self::default_path() {
                Some(p) => p,
                None => return Ok(Self::default()),
            },
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| ConfigError::Read(path.clone(), e))?;

        let trimmed = text.trim();
        let is_json =
            path.extension().and_then(|s| s.to_str()) == Some("json") || trimmed.starts_with('{');

        if is_json {
            // 1. Try standard AppConfig structure with non-empty providers:
            // { "active_provider": "...", "providers": { ... } }
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(trimmed) {
                if !cfg.providers.is_empty() {
                    return Ok(cfg);
                }
            }

            // 2. Handle both naked maps { "devin": { ... } }
            // and mixed maps { "active_provider": "devin", "devin": { ... } }
            if let Ok(serde_json::Value::Object(map)) =
                serde_json::from_str::<serde_json::Value>(trimmed)
            {
                let mut providers_map = HashMap::new();
                let mut active_provider = None;
                let mut active_model = None;
                let mut fallback_chain = Vec::new();

                for (k, v) in map {
                    if k == "active_provider" || k == "activeProvider" {
                        if let Some(s) = v.as_str() {
                            active_provider = Some(s.to_string());
                        }
                    } else if k == "active_model" || k == "activeModel" {
                        if let Some(s) = v.as_str() {
                            active_model = Some(s.to_string());
                        }
                    } else if k == "fallback_chain" || k == "fallbackChain" {
                        if let Ok(chain) = serde_json::from_value::<Vec<String>>(v) {
                            fallback_chain = chain;
                        }
                    } else if k == "providers" {
                        if let Ok(inner) =
                            serde_json::from_value::<HashMap<String, ProviderConfig>>(v)
                        {
                            providers_map.extend(inner);
                        }
                    } else if let Ok(pcfg) = serde_json::from_value::<ProviderConfig>(v) {
                        providers_map.insert(k, pcfg);
                    }
                }

                if !providers_map.is_empty() {
                    let first_key =
                        active_provider.or_else(|| providers_map.keys().next().cloned());
                    let resolved_model = active_model.or_else(|| {
                        first_key.as_ref().and_then(|k| {
                            providers_map
                                .get(k)
                                .and_then(|p| p.models.first().map(|m| m.id().to_string()))
                        })
                    });
                    return Ok(AppConfig {
                        active_provider: first_key,
                        active_model: resolved_model,
                        providers: providers_map,
                        fallback_chain,
                    });
                }
            }

            // If neither succeeded, return json parse error
            return serde_json::from_str::<AppConfig>(trimmed)
                .map_err(|e| ConfigError::Parse(path.clone(), e.to_string()));
        }

        toml::from_str(&text).map_err(|e| ConfigError::Parse(path.clone(), e.to_string()))
    }

    /// Resolve one provider's `api_key` indirection.
    pub fn resolve_secret(cfg: &ProviderConfig) -> SecretResolution {
        let raw = cfg.api_key.trim();
        if raw.is_empty() {
            return SecretResolution::Empty;
        }
        if let Some(var) = raw.strip_prefix("env:") {
            return match std::env::var(var) {
                Ok(v) if !v.is_empty() => SecretResolution::Resolved(v),
                _ => SecretResolution::Unavailable(format!("env var {var} unset or empty")),
            };
        }
        if let Some(rest) = raw.strip_prefix("keyring:") {
            let mut parts = rest.splitn(2, '/');
            let (service, account) = match (parts.next(), parts.next()) {
                (Some(s), Some(a)) if !s.is_empty() && !a.is_empty() => (s, a),
                _ => {
                    return SecretResolution::Unavailable(
                        "keyring: requires <service>/<account>".into(),
                    )
                }
            };
            return match keyring::Entry::new(service, account) {
                Ok(entry) => match entry.get_password() {
                    Ok(pw) => SecretResolution::Resolved(pw),
                    Err(e) => SecretResolution::Unavailable(format!("keyring miss: {e}")),
                },
                Err(e) => SecretResolution::Unavailable(format!("keyring init: {e}")),
            };
        }
        SecretResolution::Plaintext(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pi_agent_devin_json_config() {
        let json_content = r#"{
            "devin": {
              "baseUrl": "https://api.nyanya.moe/v1",
              "api": "openai-responses",
              "compat": {
                "supportsStore": false,
                "supportsDeveloperRole": false,
                "supportsReasoningEffort": true,
                "maxTokensField": "max_tokens"
              },
              "models": [
                {
                  "id": "devin/swe-2",
                  "name": "SWE-2",
                  "reasoning": true,
                  "input": ["text", "image"],
                  "contextWindow": 262144,
                  "cost": {
                    "input": 0,
                    "output": 0,
                    "cacheRead": 0,
                    "cacheWrite": 0
                  },
                  "thinkingLevelMap": {
                    "off": null,
                    "minimal": null,
                    "low": null,
                    "medium": "medium",
                    "high": "high",
                    "xhigh": null,
                    "max": "max"
                  }
                }
              ]
            }
        }"#;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), json_content).unwrap();

        let cfg = AppConfig::load(Some(tmp.path())).expect("should load json config");
        assert_eq!(cfg.active_provider.as_deref(), Some("devin"));
        assert_eq!(cfg.active_model.as_deref(), Some("devin/swe-2"));

        let devin = cfg.providers.get("devin").expect("devin provider exists");
        assert_eq!(devin.kind, ProviderKind::OpenaiResponses);
        assert_eq!(devin.base_url, "https://api.nyanya.moe/v1");

        let compat = devin.compat.as_ref().expect("compat should exist");
        assert_eq!(compat.supports_store, Some(false));
        assert_eq!(compat.supports_developer_role, Some(false));
        assert_eq!(compat.supports_reasoning_effort, Some(true));
        assert_eq!(compat.max_tokens_field.as_deref(), Some("max_tokens"));

        assert_eq!(devin.model_ids(), vec!["devin/swe-2"]);
        let model = devin.find_model("devin/swe-2").unwrap();
        assert_eq!(model.id(), "devin/swe-2");
        assert_eq!(model.name(), "SWE-2");

        let detailed = model.detailed().unwrap();
        assert_eq!(detailed.reasoning, Some(true));
        assert_eq!(detailed.context_window, Some(262144));
        assert_eq!(detailed.input, vec!["text", "image"]);
    }

    #[test]
    fn toml_roundtrip_preserves_provider_and_models() {
        // save_app_config writes `toml::to_string_pretty(&AppConfig)` — the
        // serialized doc must reload to the same shape (skip_serializing_if
        // keeps Option::None fields out of the doc entirely).
        let toml_doc = r#"active_provider = "devin"
active_model = "devin/swe-2"

[providers.devin]
type = "openai_responses"
base_url = "https://api.nyanya.moe/v1"
api_key = "sk-x"
default_model = "devin/swe-2"

[providers.devin.compat]
supports_reasoning_effort = true

[[providers.devin.models]]
id = "devin/swe-2"
name = "SWE-2"
reasoning = true
input = ["text", "image"]
context_window = 262144

[providers.devin.models.cost]
input = 0.0
output = 0.0

[providers.devin.models.thinking_level_map]
high = "high"
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
        std::fs::write(tmp.path(), toml_doc).unwrap();
        let cfg = AppConfig::load(Some(tmp.path())).expect("toml loads");

        let out = toml::to_string_pretty(&cfg).expect("serializes back to toml");
        let cfg2: AppConfig = toml::from_str(&out).expect("reparse");
        assert_eq!(cfg2.active_provider.as_deref(), Some("devin"));
        let devin = cfg2.providers.get("devin").unwrap();
        assert_eq!(devin.kind, ProviderKind::OpenaiResponses);
        let m = devin.find_model("devin/swe-2").unwrap().detailed().unwrap();
        assert_eq!(m.reasoning, Some(true));
        assert_eq!(m.context_window, Some(262144));
        assert_eq!(m.input, vec!["text", "image"]);
        assert_eq!(m.cost.as_ref().unwrap().input, Some(0.0));
        assert_eq!(
            m.thinking_level_map
                .as_ref()
                .unwrap()
                .get("high")
                .unwrap()
                .as_deref(),
            Some("high")
        );
    }
}
