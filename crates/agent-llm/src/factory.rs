//! `ProviderFactory` — `ProviderConfig` → `Arc<dyn LlmProvider>`.
//!
//! Keys on `type`; unknown type is a config error at load, not first request.
//! Missing/unsolvable secrets mark the provider unavailable (never panic):
//! `build` returns `Err` carrying the reason so the UI can grey it out.

use std::sync::Arc;

use thiserror::Error;

use crate::adapters::{
    anthropic::AnthropicProvider, GenericOpenAiProvider, OpenAiResponsesProvider,
};
use crate::config::{ModelConfig, ProviderConfig, ProviderKind, SecretResolution};
use crate::provider::{LlmProvider, ModelParams};

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),

    #[error("provider construction failed: {0}")]
    Build(#[from] anyhow::Error),
}

pub struct ProviderFactory;

impl ProviderFactory {
    /// Build a provider from config. Resolves `env:`/`keyring:` indirections;
    /// plaintext keys warn but still build.
    pub fn build(cfg: &ProviderConfig) -> Result<Arc<dyn LlmProvider>, FactoryError> {
        let key = match crate::config::AppConfig::resolve_secret(cfg) {
            SecretResolution::Resolved(s) | SecretResolution::Plaintext(s) => s,
            SecretResolution::Unavailable(why) => {
                return Err(FactoryError::Unavailable(why));
            }
            SecretResolution::Empty => String::new(),
        };
        if matches!(
            crate::config::AppConfig::resolve_secret(cfg),
            SecretResolution::Plaintext(_)
        ) {
            tracing::warn!(
                base_url = %cfg.base_url,
                "api_key is plaintext in config.toml — prefer env: or keyring: indirection"
            );
        }

        match cfg.kind {
            ProviderKind::OpenaiCompat => {
                let mut p =
                    GenericOpenAiProvider::new(&cfg.base_url, key).map_err(FactoryError::Build)?;
                for (k, v) in &cfg.headers {
                    p = p.with_header(k.clone(), v.clone());
                }
                if let Some(compat) = &cfg.compat {
                    p = p.with_compat(compat.clone());
                }
                Ok(Arc::new(p))
            }
            ProviderKind::OpenaiResponses => {
                let mut p = OpenAiResponsesProvider::new(&cfg.base_url, key)
                    .map_err(FactoryError::Build)?;
                for (k, v) in &cfg.headers {
                    p = p.with_header(k.clone(), v.clone());
                }
                if let Some(compat) = &cfg.compat {
                    p = p.with_compat(compat.clone());
                }
                Ok(Arc::new(p))
            }
            ProviderKind::Anthropic => {
                let mut p =
                    AnthropicProvider::new(&cfg.base_url, key).map_err(FactoryError::Build)?;
                for (k, v) in &cfg.headers {
                    p = p.with_header(k.clone(), v.clone());
                }
                Ok(Arc::new(p))
            }
            ProviderKind::Gemini => {
                let mut p = crate::adapters::gemini::GeminiProvider::new(&cfg.base_url, key)
                    .map_err(FactoryError::Build)?;
                for (k, v) in &cfg.headers {
                    p = p.with_header(k.clone(), v.clone());
                }
                Ok(Arc::new(p))
            }
        }
    }

    /// The per-model wire settings for `model_id` under `cfg` — `maxTokens`,
    /// `samplingParams`, and the model's `compat` merged over the provider's.
    ///
    /// Returns `ModelParams::EMPTY` for a model the config doesn't describe, so
    /// an unconfigured model keeps every adapter default.
    pub fn model_params(cfg: &ProviderConfig, model_id: &str) -> ModelParams {
        if cfg.find_model(model_id).is_none() {
            return ModelParams::EMPTY;
        }
        // `model_opts` folds in `modelOverrides`; a bare string entry yields a
        // settings-free `ModelConfig`, which is exactly `EMPTY`.
        Self::model_params_from(&cfg.model_opts(model_id))
    }

    /// Same, from an already-resolved [`ModelConfig`] (i.e. with
    /// `modelOverrides` applied) — what the engine holds after a model switch.
    pub fn model_params_from(model: &ModelConfig) -> ModelParams {
        ModelParams {
            max_tokens: model.max_tokens.map(|v| v.min(u32::MAX as u64) as u32),
            sampling_params: model.sampling_params.clone(),
            // The model's own `compat`; the provider's is merged in by the
            // adapter at request time (`ModelParams::compat_with`), so a
            // provider-level change still reaches a resolved model.
            compat: model.compat.clone(),
        }
    }
}
