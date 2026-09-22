//! agent-llm — pluggable LLM provider layer.
//!
//! The kernel and UI never see vendor JSON: everything behind `Arc<dyn LlmProvider>`
//! speaks the normalized `StreamChunk` protocol, so adding a provider means adding
//! one adapter file.


pub mod adapters;
pub mod config;
pub mod factory;
pub mod masking;
pub mod provider;
pub mod sampler;
pub mod sse;
pub mod transport;
pub mod types;

pub use config::{
    AppConfig, ConfigWarning, CostTier, ImageLimits, ImageResize, InputLimits, ModelConfig,
    ModelCost, ModelEntry, ModelOverride, PromptCache, ProviderCompat, ProviderConfig, ProviderKind,
    THINKING_LEVELS,
};
pub use factory::ProviderFactory;
pub use masking::EgressMasker;
pub use provider::{BoxStream, Capabilities, LlmProvider, ModelParams};
pub use transport::{DoneGuard, Transport};
pub use types::{ChatMessage, Role, StreamChunk, ToolCall};
