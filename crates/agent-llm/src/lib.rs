//! agent-llm — pluggable LLM provider layer.
//!
//! The kernel and UI never see vendor JSON: everything behind `Arc<dyn LlmProvider>`
//! speaks the normalized `StreamChunk` protocol, so adding a provider means adding
//! one wire entry in the rig bridge.

pub mod cache_warm;
mod codex_transport;
pub mod config;
pub mod factory;
mod responses_state;
mod rig_bridge;
pub mod usage;

// The contract lives in `agent-llm-core`; re-exported so the module paths
// (`agent_llm::types`, `agent_llm::provider`, …) keep resolving.
pub use agent_llm_core::{compat, masking, provider, sampler, types};

pub use agent_llm_core::{
    BoxStream, Capabilities, EgressMasker, LlmProvider, ModelParams, ProviderCompat,
    UnavailableProvider,
};
pub use agent_llm_core::{ChatMessage, ProviderState, Role, StreamChunk, ToolCall};
pub use cache_warm::{WarmingControl, WarmingProvider};
pub use config::{
    AppConfig, ConfigWarning, CostTier, ImageLimits, ImageResize, InputLimits, ModelConfig,
    ModelCost, ModelEntry, ModelOverride, PromptCache, ProviderConfig, ProviderKind,
    THINKING_LEVELS,
};
pub use factory::ProviderFactory;
pub use rig_bridge::RigProvider;
pub use usage::UsageObserver;
