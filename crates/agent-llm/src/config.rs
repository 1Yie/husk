//! `config.toml` schema + secret indirection + hot-reload hooks.
//!
//! `~/.config/<app>/config.toml` (llm-provider-layer.md §Config):
//!
//! ```toml
//! active_provider = "grok"
//! active_model    = "grok-4"
//!
//! [providers.grok]
//! type     = "openai_compat"
//! base_url = "https://api.x.ai/v1"
//! api_key  = "env:XAI_API_KEY"      # env: indirection — never force plaintext
//!
//! [providers.claude]
//! type     = "anthropic"
//! base_url = "https://api.anthropic.com/v1"
//! api_key  = "keyring:agent-rs/claude"   # OS credential store
//!
//! fallback_chain = ["ollama"]
//! ```
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
    #[serde(default)]
    pub active_provider: Option<String>,
    #[serde(default)]
    pub active_model: Option<String>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// Degrade chain: on unrecoverable 429/5xx, next provider takes over.
    #[serde(default)]
    pub fallback_chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub kind: ProviderKind,
    pub base_url: String,
    /// `env:VAR` | `keyring:<service>/<account>` | plaintext (warns).
    #[serde(default)]
    pub api_key: String,
    /// Optional static headers (e.g. `{"HTTP-Referer" = "…"}`).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Defaults per-provider (rare) — model usually comes from `active_model`.
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenaiCompat,
    OpenaiResponses,
    Anthropic,
    Mock,
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
    /// Default config path: `~/.config/agent-rs/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("agent-rs").join("config.toml"))
    }

    /// Load from disk; missing file yields `AppConfig::default()` (first-run).
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
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError::Read(path.clone(), e))?;
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
