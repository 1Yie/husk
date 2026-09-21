//! agent-llm — pluggable LLM provider layer.
//!
//! Design goal (llm-provider-layer.md): **the kernel and UI never see vendor
//! JSON.** Everything behind `Arc<dyn LlmProvider>` speaks the normalized
//! `StreamChunk` protocol; adding a provider = adding one adapter file.
//!
//! Stage 2 scope: types, trait, OpenAI-compat adapter, mock, config,
//! factory, SSE helpers, sampler resilience. `masking`/`fallback` are stubbed
//! for their Stage-11/2 completion.

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
    AppConfig, ModelConfig, ModelCost, ModelEntry, ProviderCompat, ProviderConfig, ProviderKind,
};
pub use factory::ProviderFactory;
pub use masking::EgressMasker;
pub use provider::{BoxStream, Capabilities, LlmProvider};
pub use transport::{DoneGuard, Transport};
pub use types::{ChatMessage, Role, StreamChunk, ToolCall};
