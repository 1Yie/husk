//! Vendor adapters — the only place vendor JSON shapes exist.

pub mod anthropic;
pub mod mock;
pub mod openai_compat;

pub use mock::MockProvider;
pub use openai_compat::GenericOpenAiProvider;
