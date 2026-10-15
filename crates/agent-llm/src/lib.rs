//! agent-llm — pluggable LLM provider layer.
//!
//! The kernel and UI never see vendor JSON: everything behind `Arc<dyn LlmProvider>`
//! speaks the normalized `StreamChunk` protocol, so adding a provider means adding
//! one wire entry in the rig bridge.

pub mod config;
pub mod factory;
mod rig_bridge;

// The contract lives in `agent-llm-core`; re-exported so the module paths
// (`agent_llm::types`, `agent_llm::provider`, …) keep resolving.
pub use agent_llm_core::{compat, masking, provider, sampler, types};

pub use agent_llm_core::{
    BoxStream, Capabilities, EgressMasker, LlmProvider, ModelParams, ProviderCompat,
    UnavailableProvider,
};
pub use agent_llm_core::{ChatMessage, Role, StreamChunk, ToolCall};
pub use config::{
    AppConfig, ConfigWarning, CostTier, ImageLimits, ImageResize, InputLimits, ModelConfig,
    ModelCost, ModelEntry, ModelOverride, PromptCache, ProviderConfig, ProviderKind,
    THINKING_LEVELS,
};
pub use factory::ProviderFactory;
pub use rig_bridge::RigProvider;
