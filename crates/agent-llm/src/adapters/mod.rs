//! Vendor adapters — the only place vendor JSON shapes exist.

pub mod anthropic;
pub mod gemini;
pub mod mock;
pub mod openai_compat;
pub mod openai_responses;

pub use mock::MockProvider;
pub use openai_compat::GenericOpenAiProvider;
pub use openai_responses::OpenAiResponsesProvider;
