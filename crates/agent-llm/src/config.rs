//! `config.toml` / `config.json` schema + secret indirection + hot-reload hooks.
//!
//! Supports both traditional TOML config and modern pi-agent / Devin compatible JSON formats.
//!
//! Resolution rules: `env:VAR` reads process env; `keyring:<service>/<account>`
//! reads the OS credential store; anything else is plaintext (warns at load).
//! A missing source marks the provider `available: false` — never panic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use agent_llm_core::compat::ProviderCompat;
pub use agent_llm_core::thinking::{
    available_thinking_levels, normalize_thinking_level, THINKING_LEVELS,
};
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
        rename = "openai_completions",
        alias = "openai-completions",
        alias = "openai_compat"
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
    /// The config file in `config_dir`: `config.toml` when it exists, else
    /// `config.json` when that exists, else the `config.toml` path to create.
    ///
    /// TOML is canonical — a hand-edited `config.toml` is ALWAYS the live file
    /// when it exists; `config.json` only fills in otherwise. Directory
    /// migration (a product renaming itself) is the host's job, not this one's.
    pub fn default_path(config_dir: &Path) -> PathBuf {
        let toml_path = config_dir.join("config.toml");
        if toml_path.exists() {
            return toml_path;
        }
        let json_path = config_dir.join("config.json");
        if json_path.exists() {
            return json_path;
        }
        toml_path
    }

    /// Load from disk; missing file yields `AppConfig::default()` (first-run).
    /// `file` overrides the `config_dir` lookup. Supports both JSON and TOML
    /// syntax, and naked provider mappings.
    pub fn load(config_dir: &Path, file: Option<&Path>) -> Result<Self, ConfigError> {
        let path = match file {
            Some(p) => p.to_path_buf(),
            None => Self::default_path(config_dir),
        };
        Self::load_file(&path)
    }

    /// Load one config file; a missing file yields `AppConfig::default()`.
    fn load_file(path: &Path) -> Result<Self, ConfigError> {
        let path = path.to_path_buf();
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

    /// Write `cfg` to `path` in the file's own format (TOML stays TOML).
    ///
    /// Guards against silent data loss:
    /// - an existing file that no longer parses is never overwritten — the
    ///   user may have a hand edit in flight;
    /// - TOML is merged into the file in place, so comments and keys the schema
    ///   does not model survive the save;
    /// - the previous file is copied to `<name>.<unix-ms>.bak` first, and only
    ///   the newest [`BACKUP_KEEP`] backups are kept;
    /// - the new content lands via temp-file + rename, so a crash mid-write
    ///   leaves either the old or the new file, never a truncated one.
    pub fn save_to(path: &Path, cfg: &AppConfig) -> Result<(), ConfigError> {
        let is_toml = path.extension().and_then(|e| e.to_str()) == Some("toml");
        let text = if path.exists() {
            // A file we cannot read is a file we must not replace.
            Self::load_file(path)?;
            if is_toml {
                let existing = std::fs::read_to_string(path)
                    .map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
                merge_into_toml(&existing, cfg)
                    .map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?
            } else {
                serde_json::to_string_pretty(cfg)
                    .map_err(|e| ConfigError::Parse(path.to_path_buf(), e.to_string()))?
            }
        } else if is_toml {
            toml::to_string_pretty(cfg)
                .map_err(|e| ConfigError::Parse(path.to_path_buf(), e.to_string()))?
        } else {
            serde_json::to_string_pretty(cfg)
                .map_err(|e| ConfigError::Parse(path.to_path_buf(), e.to_string()))?
        };
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        if path.exists() {
            backup_previous(path).map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        }
        let mut tmp = tempfile::NamedTempFile::new_in(dir)
            .map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        std::io::Write::write_all(&mut tmp, text.as_bytes())
            .map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        tmp.as_file()
            .sync_all()
            .map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        tmp.persist(path)
            .map_err(|e| ConfigError::Read(path.to_path_buf(), e.error))?;
        Ok(())
    }

    /// Real key material for every provider that resolves — the values the
    /// egress masker must scrub from outbound requests. Uses the same
    /// resolution as `ProviderFactory::build`, so the masker and the wire
    /// never disagree about which strings are secrets.
    pub fn resolved_secrets(&self) -> Vec<String> {
        self.providers
            .values()
            .filter_map(|p| match Self::resolve_secret(p) {
                SecretResolution::Resolved(s) | SecretResolution::Plaintext(s) => Some(s),
                SecretResolution::Unavailable(_) | SecretResolution::Empty => None,
            })
            .collect()
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

/// How many timestamped `.bak` copies of a config file to keep.
pub const BACKUP_KEEP: usize = 5;

/// Merge `cfg` into an existing TOML document, keeping its comments, layout,
/// and any key the schema does not model. Tables merge recursively, so a
/// comment attached to `api_key` survives when only `base_url` changes.
fn merge_into_toml(existing: &str, cfg: &AppConfig) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = existing.parse().map_err(|e| format!("{e}"))?;
    let fresh = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let fresh: toml_edit::DocumentMut = fresh.parse().map_err(|e| format!("{e}"))?;
    prune_keyed(doc.as_table_mut(), fresh.as_table());
    merge_table(doc.as_table_mut(), fresh.as_table());
    Ok(doc.to_string())
}

/// Maps whose keys are user-chosen ids, not schema fields: `providers`, and
/// inside each provider its `headers` / `model_overrides`. The config owns
/// every entry, so an id missing from the new config was renamed or deleted
/// and must leave the file too — merging alone would keep the old entry and
/// a rename would show up as a second provider.
fn prune_keyed(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    fn prune(dst: &mut toml_edit::Table, src: &toml_edit::Table, key: &str) {
        let Some(d) = dst.get_mut(key).and_then(|i| i.as_table_like_mut()) else {
            return;
        };
        let keep = src.get(key).and_then(|i| i.as_table_like());
        let gone: Vec<String> = d
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| !keep.is_some_and(|s| s.contains_key(k)))
            .collect();
        for k in gone {
            d.remove(&k);
        }
    }
    prune(dst, src, "providers");
    let (Some(dp), Some(sp)) = (
        dst.get_mut("providers").and_then(|i| i.as_table_mut()),
        src.get("providers").and_then(|i| i.as_table()),
    ) else {
        return;
    };
    for (id, d) in dp.iter_mut() {
        let (Some(d), Some(s)) = (
            d.as_table_mut(),
            sp.get(id.get()).and_then(|i| i.as_table()),
        ) else {
            continue;
        };
        prune(d, s, "headers");
        prune(d, s, "model_overrides");
    }
}

fn merge_table(dst: &mut toml_edit::Table, src: &toml_edit::Table) {
    // Keys the schema no longer emits (e.g. an emptied optional) are removed,
    // but only those the schema itself owns; unknown keys are not in `src`
    // and are left alone by never being visited here.
    for (key, item) in src.iter() {
        match (dst.get_mut(key), item) {
            (Some(toml_edit::Item::Table(d)), toml_edit::Item::Table(s)) => merge_table(d, s),
            (Some(toml_edit::Item::Value(d)), toml_edit::Item::Value(s)) => {
                // Replace the value, keep the decor (the comment around it).
                let decor = d.decor().clone();
                *d = s.clone();
                *d.decor_mut() = decor;
            }
            _ => {
                dst.insert(key, item.clone());
            }
        }
    }
}

/// Copy the current file to `<name>.<unix-ms>.bak`, then prune the oldest
/// backups beyond [`BACKUP_KEEP`].
fn backup_previous(path: &Path) -> std::io::Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    // Two saves in the same millisecond must not share a backup name, or the
    // earlier copy is silently overwritten. Bump the stamp until it is free.
    let mut stamp = stamp;
    let mut target = dir.join(format!("{name}.{stamp}.bak"));
    while target.exists() {
        stamp += 1;
        target = dir.join(format!("{name}.{stamp}.bak"));
    }
    std::fs::copy(path, &target)?;

    let prefix = format!("{name}.");
    let mut baks: Vec<std::path::PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
                .unwrap_or(false)
        })
        .collect();
    // Stamps are fixed-width-enough millisecond counts; sort numerically.
    baks.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix(&prefix))
            .and_then(|n| n.strip_suffix(".bak"))
            .and_then(|n| n.parse::<u128>().ok())
            .unwrap_or(0)
    });
    let excess = baks.len().saturating_sub(BACKUP_KEEP);
    for old in baks.iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_levels_are_normalized_against_the_model_allowlist() {
        let map = HashMap::from([
            ("low".into(), Some("low".into())),
            ("high".into(), Some("high".into())),
            ("max".into(), Some("max".into())),
            ("medium".into(), None),
            ("xhigh".into(), Some(String::new())),
            ("custom".into(), Some("custom".into())),
        ]);
        assert_eq!(
            available_thinking_levels(Some(&map)),
            ["off", "low", "high", "max"]
        );
        assert_eq!(
            normalize_thinking_level(Some("xhigh"), Some(&map)).as_deref(),
            Some("low")
        );
        assert_eq!(
            normalize_thinking_level(Some(" HIGH "), Some(&map)).as_deref(),
            Some("high")
        );
        assert_eq!(
            normalize_thinking_level(Some("off"), Some(&map)).as_deref(),
            Some("off")
        );
        assert_eq!(
            normalize_thinking_level(None, Some(&map)).as_deref(),
            Some("low")
        );
        assert_eq!(normalize_thinking_level(Some("xhigh"), None), None);
        assert_eq!(available_thinking_levels(None), Vec::<String>::new());
        assert_eq!(
            normalize_thinking_level(Some("max"), Some(&HashMap::new())).as_deref(),
            Some("off")
        );
    }

    #[test]
    fn thinking_levels_respect_effective_model_overrides() {
        let provider: ProviderConfig = serde_json::from_value(serde_json::json!({
            "type": "openai_compat", "base_url": "https://example.invalid", "api_key": "test",
            "models": [{ "id": "a", "thinking_level_map": { "low": "low", "high": "high", "xhigh": "xhigh" } }],
            "model_overrides": { "a": { "thinking_level_map": { "xhigh": null, "max": "max" } } }
        })).unwrap();
        let effective = provider.model_opts("a");
        assert_eq!(
            available_thinking_levels(effective.thinking_level_map.as_ref()),
            ["off", "low", "high", "max"]
        );
        assert_eq!(
            normalize_thinking_level(Some("xhigh"), effective.thinking_level_map.as_ref())
                .as_deref(),
            Some("low")
        );
    }

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

        let cfg = AppConfig::load_file(tmp.path()).expect("should load json config");
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
        let cfg = AppConfig::load_file(tmp.path()).expect("toml loads");

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

    /// README 的写法 `kind = "openai_compat"` 必须可读，否则整份配置报废。
    #[test]
    fn readme_kind_alias_loads() {
        let doc = "active_provider = \"deepseek\"\n\
            [providers.deepseek]\n\
            kind = \"openai_compat\"\n\
            base_url = \"https://api.deepseek.com\"\n\
            api_key = \"env:X\"\n";
        let tmp = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
        std::fs::write(tmp.path(), doc).unwrap();
        let cfg = AppConfig::load_file(tmp.path()).expect("README alias must load");
        assert_eq!(cfg.providers["deepseek"].kind, ProviderKind::OpenaiCompat);
    }

    /// 一个坏字段不能让 `load` 悄悄返回空配置：必须是 `Err`，并且带文件路径。
    #[test]
    fn bad_field_is_a_located_error_not_an_empty_config() {
        let doc = "[providers.a]\n\
            kind = \"openai_completions\"\n\
            base_url = \"https://x\"\n\
            [[providers.a.models]]\n\
            id = \"m\"\n\
            context_window = 1.5\n";
        let tmp = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
        std::fs::write(tmp.path(), doc).unwrap();
        let err = AppConfig::load_file(tmp.path()).expect_err("bad config must not load");
        let msg = err.to_string();
        assert!(msg.contains("failed to parse config file"), "{msg}");
        assert!(msg.contains(&tmp.path().display().to_string()), "{msg}");
    }

    /// 保存时先留 `.bak`，再原子替换：旧内容必须可以找回。
    #[test]
    fn save_keeps_previous_file_as_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "active_provider = \"old\"\n").unwrap();
        let cfg = AppConfig {
            active_provider: Some("new".into()),
            ..Default::default()
        };
        AppConfig::save_to(&path, &cfg).expect("save");
        let bak = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().ends_with(".bak"))
            .expect("a timestamped backup exists");
        assert_eq!(
            std::fs::read_to_string(bak).unwrap(),
            "active_provider = \"old\"\n"
        );
        let now: AppConfig = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(now.active_provider.as_deref(), Some("new"));
    }

    /// 已有文件无法解析时拒绝覆盖，避免用户手改的配置被空文档替换。
    #[test]
    fn save_refuses_to_overwrite_unparseable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let broken = "[providers.a\nthis is not toml";
        std::fs::write(&path, broken).unwrap();
        let cfg = AppConfig::default();
        assert!(AppConfig::save_to(&path, &cfg).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    /// Egress masking must see the real key material of every configured
    /// provider. A `keyring:`/`env:` handle is not a secret and must not be
    /// returned; a missing source contributes nothing.
    #[test]
    fn resolved_secrets_collects_real_key_material_only() {
        std::env::set_var("RESOLVED_SECRET_TEST_KEY", "sk-live-abcdef12");
        let mut providers = HashMap::new();
        providers.insert(
            "envd".to_string(),
            ProviderConfig {
                api_key: "env:RESOLVED_SECRET_TEST_KEY".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "plain".to_string(),
            ProviderConfig {
                api_key: "plaintext-key-9999".into(),
                ..Default::default()
            },
        );
        providers.insert(
            "missing".to_string(),
            ProviderConfig {
                api_key: "env:DEFINITELY_UNSET_MASK_VAR".into(),
                ..Default::default()
            },
        );
        let cfg = AppConfig {
            providers,
            ..Default::default()
        };
        let secrets = cfg.resolved_secrets();
        assert!(
            secrets.contains(&"sk-live-abcdef12".to_string()),
            "{secrets:?}"
        );
        assert!(
            secrets.contains(&"plaintext-key-9999".to_string()),
            "{secrets:?}"
        );
        assert!(
            !secrets.iter().any(|s| s.starts_with("env:")),
            "{secrets:?}"
        );
        assert_eq!(secrets.len(), 2, "{secrets:?}");
    }

    /// A hand-written comment and a key the schema does not model must survive
    /// a save. Re-serializing the whole struct drops both.
    #[test]
    fn save_preserves_comments_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "# my deepseek setup, do not delete\n\
            active_provider = \"deepseek\"\n\
            my_note = \"keep me\"\n\
            \n\
            [providers.deepseek]\n\
            kind = \"openai_completions\"\n\
            base_url = \"https://api.deepseek.com\"\n\
            api_key = \"env:DEEPSEEK_API_KEY\"  # rotate yearly\n";
        std::fs::write(&path, original).unwrap();

        let mut cfg = AppConfig::load_file(&path).unwrap();
        cfg.active_model = Some("deepseek-chat".into());
        AppConfig::save_to(&path, &cfg).unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# my deepseek setup, do not delete"), "{out}");
        assert!(out.contains("# rotate yearly"), "{out}");
        assert!(out.contains("my_note = \"keep me\""), "{out}");
        assert!(out.contains("active_model = \"deepseek-chat\""), "{out}");
    }

    /// Renaming a provider in the settings writes the config under the new id.
    /// The old table must leave the file — merging alone kept it, and the
    /// rename showed up as a second, duplicate provider. Deleting works the
    /// same way. Comments on surviving providers and unknown top-level keys
    /// still stay.
    #[test]
    fn save_drops_renamed_and_deleted_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "my_note = \"keep me\"\n\
            \n\
            [providers.OldName]\n\
            kind = \"openai_completions\"\n\
            base_url = \"https://old\"\n\
            \n\
            [providers.gone]\n\
            kind = \"openai_completions\"\n\
            base_url = \"https://gone\"\n\
            \n\
            # keep this comment\n\
            [providers.stay]\n\
            kind = \"openai_completions\"\n\
            base_url = \"https://stay\"\n\
            [providers.stay.headers]\n\
            X-Old = \"1\"\n";
        std::fs::write(&path, original).unwrap();

        let mut cfg = AppConfig::load_file(&path).unwrap();
        let renamed = cfg.providers.remove("OldName").unwrap();
        cfg.providers.insert("NewName".into(), renamed);
        cfg.providers.remove("gone");
        cfg.providers.get_mut("stay").unwrap().headers.clear();
        AppConfig::save_to(&path, &cfg).unwrap();

        let back = AppConfig::load_file(&path).unwrap();
        let mut ids: Vec<_> = back.providers.keys().cloned().collect();
        ids.sort();
        assert_eq!(ids, vec!["NewName", "stay"]);
        assert_eq!(back.providers["NewName"].base_url, "https://old");
        assert!(back.providers["stay"].headers.is_empty());
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# keep this comment"), "{out}");
        assert!(out.contains("my_note = \"keep me\""), "{out}");
    }

    /// Every save keeps a timestamped backup, and only the newest few are kept.
    #[test]
    fn backups_are_timestamped_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "active_provider = \"a\"\n").unwrap();
        let cfg = AppConfig {
            active_provider: Some("b".into()),
            ..Default::default()
        };
        for _ in 0..(BACKUP_KEEP + 3) {
            AppConfig::save_to(&path, &cfg).unwrap();
        }
        let mut baks: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("config.toml.") && n.ends_with(".bak"))
            .collect();
        baks.sort();
        assert_eq!(baks.len(), BACKUP_KEEP, "{baks:?}");
        assert!(!std::fs::read_to_string(dir.path().join(&baks[0]))
            .unwrap()
            .is_empty());
    }

    /// The degrade chain must survive a save: the settings UI writes it through
    /// `save_to`, and a dropped chain would silently disable fallback.
    #[test]
    fn fallback_chain_round_trips_through_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "active_provider = \"a\"\n").unwrap();
        let mut cfg = AppConfig {
            active_provider: Some("a".into()),
            fallback_chain: vec!["b".into(), "c".into()],
            ..Default::default()
        };
        cfg.providers.insert(
            "a".into(),
            ProviderConfig {
                base_url: "https://a".into(),
                ..Default::default()
            },
        );
        AppConfig::save_to(&path, &cfg).unwrap();
        let back = AppConfig::load_file(&path).unwrap();
        assert_eq!(back.fallback_chain, vec!["b", "c"]);
    }
}
