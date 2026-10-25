//! `ProviderFactory` — `ProviderConfig` → `Arc<dyn LlmProvider>`.
//!
//! Keys on `type`; unknown type is a config error at load, not first request.
//! Missing/unsolvable secrets mark the provider unavailable (never panic):
//! `build` returns `Err` carrying the reason so the UI can grey it out.

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::config::{ModelConfig, ProviderConfig, SecretResolution};
use crate::provider::{LlmProvider, ModelParams};
use crate::rig_bridge::RigProvider;

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),

    #[error("provider construction failed: {0}")]
    Build(#[from] anyhow::Error),
}

/// The `User-Agent` every request advertises. Standalone gateways (OpenCode
/// Go, …) route, meter and abuse-filter by client, and ask a client to name
/// itself instead of wearing an SDK's or an HTTP library's default. A provider
/// whose deployment wants its own name declares `userAgent` in config.
///
/// The version is the shipping binary's, not this crate's: each front end
/// declares itself once at startup ([`set_client_user_agent`]). Until then the
/// bare client name goes out — no version beats a wrong one.
pub fn client_user_agent() -> &'static str {
    CLIENT_USER_AGENT.get().map_or("husk", String::as_str)
}

/// Declare the client identity for this process, e.g.
/// `concat!("husk/", env!("CARGO_PKG_VERSION"))` from the binary crate. The
/// first call wins; later calls are ignored.
pub fn set_client_user_agent(user_agent: impl Into<String>) {
    let _ = CLIENT_USER_AGENT.set(user_agent.into());
}

static CLIENT_USER_AGENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// One `reqwest::Client` shared by every rig client — rustls-only TLS,
/// generous total timeout for slow generations, short connect timeout.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(client_user_agent())
        .timeout(Duration::from_secs(30 * 60))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client")
}

pub struct ProviderFactory;

impl ProviderFactory {
    /// Build a provider from config. Resolves `env:`/`keyring:` indirections;
    /// plaintext keys warn but still build.
    pub fn build(cfg: &ProviderConfig) -> Result<Arc<dyn LlmProvider>, FactoryError> {
        Self::build_with_conversation(cfg, None, None)
    }

    /// Build with a per-session cache-write sink. Only the host's foreground
    /// session provider needs one; background providers (distiller, child
    /// engines) pass `None`. This mints its own routing key — a caller that
    /// serves an existing conversation wants [`Self::build_with_conversation`],
    /// not this.
    pub fn build_with_observer(
        cfg: &ProviderConfig,
        observer: Option<Arc<crate::usage::UsageObserver>>,
    ) -> Result<Arc<dyn LlmProvider>, FactoryError> {
        Self::build_with_conversation(cfg, observer, None)
    }

    /// Build the provider one conversation runs on. `conversation` is that
    /// conversation's opaque identity — the host derives it from the persisted
    /// session id — and every request carries a routing key derived from it
    /// (`prompt_cache_key`, session-affinity headers). Rebuilding the provider
    /// inside the same conversation (model switch, fallback chain, delegated
    /// child) therefore keeps the gateway's cache routing instead of starting
    /// cold on a fresh key.
    ///
    /// `None` mints a random key: right for one-off work whose requests never
    /// share a prompt prefix with anything else.
    pub fn build_with_conversation(
        cfg: &ProviderConfig,
        observer: Option<Arc<crate::usage::UsageObserver>>,
        conversation: Option<&str>,
    ) -> Result<Arc<dyn LlmProvider>, FactoryError> {
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
        RigProvider::new(cfg, key, http_client(), observer, conversation)
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
            .map_err(FactoryError::Build)
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
        let compat = if model.reasoning == Some(false) {
            // A model capability is stronger than a caller's effort hint or
            // the protocol's reasoning-enabled baseline. No model-name guesses.
            let mut compat = model.compat.clone().unwrap_or_default();
            compat.supports_reasoning_effort = Some(false);
            Some(compat)
        } else {
            model.compat.clone()
        };
        ModelParams {
            max_tokens: model.max_tokens.map(|v| v.min(u32::MAX as u64) as u32),
            sampling_params: model.sampling_params.clone(),
            // The model's own `compat`; the provider's is merged in by the
            // provider at request time (`ModelParams::compat_with`), so a
            // provider-level change still reaches a resolved model.
            compat,
        }
    }
}
