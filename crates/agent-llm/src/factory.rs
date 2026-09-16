//! `ProviderFactory` — `ProviderConfig` → `Arc<dyn LlmProvider>`.
//!
//! Keys on `type`; unknown type is a config error at load, not first request.
//! Missing/unsolvable secrets mark the provider unavailable (never panic):
//! `build` returns `Err` carrying the reason so the UI can grey it out.

use std::sync::Arc;

use thiserror::Error;

use crate::adapters::{anthropic::AnthropicProvider, GenericOpenAiProvider, MockProvider};
use crate::config::{ProviderConfig, ProviderKind, SecretResolution};
use crate::provider::LlmProvider;

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
                let mut p = GenericOpenAiProvider::new(&cfg.base_url, key)
                    .map_err(FactoryError::Build)?;
                for (k, v) in &cfg.headers {
                    p = p.with_header(k.clone(), v.clone());
                }
                Ok(Arc::new(p))
            }
            ProviderKind::Anthropic => Err(FactoryError::Unavailable(
                AnthropicProvider::unimplemented().to_string(),
            )),
            ProviderKind::Mock => Ok(Arc::new(MockProvider::new())),
        }
    }
}
