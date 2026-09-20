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
    #[serde(default, alias = "activeProvider")]
    pub active_provider: Option<String>,
    #[serde(default, alias = "activeModel")]
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
    #[serde(default, alias = "default_model", alias = "defaultModel")]
    pub default_model: Option<String>,
    /// Compatibility flags (matching pi-agent / Devin format).
    #[serde(default)]
    pub compat: Option<ProviderCompat>,
    /// Configured models for this provider (can be simple string IDs or rich objects).
    #[serde(default)]
    pub models: Vec<ModelEntry>,
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
}

/// Feature compatibility flags matching pi-agent / Devin configs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCompat {
    #[serde(default, alias = "supports_store", alias = "supportsStore")]
    pub supports_store: Option<bool>,
    #[serde(default, alias = "supports_developer_role", alias = "supportsDeveloperRole")]
    pub supports_developer_role: Option<bool>,
    #[serde(default, alias = "supports_reasoning_effort", alias = "supportsReasoningEffort")]
    pub supports_reasoning_effort: Option<bool>,
    #[serde(default, alias = "max_tokens_field", alias = "maxTokensField")]
    pub max_tokens_field: Option<String>,
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

/// Rich model definition matching pi-agent schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default, alias = "context_window", alias = "contextWindow")]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub cost: Option<ModelCost>,
    #[serde(default, alias = "thinking_level_map", alias = "thinkingLevelMap")]
    pub thinking_level_map: Option<HashMap<String, Option<String>>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default, alias = "cache_read", alias = "cacheRead")]
    pub cache_read: Option<f64>,
    #[serde(default, alias = "cache_write", alias = "cacheWrite")]
    pub cache_write: Option<f64>,
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
    #[serde(rename = "mock")]
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
        let json_path = dir.join("config.json");
        if json_path.exists() {
            return Some(json_path);
        }
        let toml_path = dir.join("config.toml");
        if toml_path.exists() {
            return Some(toml_path);
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
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError::Read(path.clone(), e))?;

        let trimmed = text.trim();
        let is_json = path.extension().and_then(|s| s.to_str()) == Some("json")
            || trimmed.starts_with('{');

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
            if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(trimmed) {
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
                        if let Ok(inner) = serde_json::from_value::<HashMap<String, ProviderConfig>>(v) {
                            providers_map.extend(inner);
                        }
                    } else if let Ok(pcfg) = serde_json::from_value::<ProviderConfig>(v) {
                        providers_map.insert(k, pcfg);
                    }
                }

                if !providers_map.is_empty() {
                    let first_key = active_provider.or_else(|| providers_map.keys().next().cloned());
                    let resolved_model = active_model.or_else(|| {
                        first_key.as_ref().and_then(|k| {
                            providers_map.get(k).and_then(|p| p.models.first().map(|m| m.id().to_string()))
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
}
