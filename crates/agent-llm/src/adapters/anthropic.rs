//! Anthropic adapter — **stub until v2** (llm-provider-layer.md).
//!
//! The wire protocol differs (`content_block_*` events, `input_json_delta`,
//! `thinking` blocks, `x-api-key` + `anthropic-version` headers, mandatory
//! `max_tokens`). It lands behind the same `LlmProvider` trait so the kernel
//! never learns it exists.

use async_trait::async_trait;

use crate::provider::{BoxStream, LlmProvider};
use crate::types::{ChatMessage, StreamChunk};

/// Placeholder so `ProviderKind::Anthropic` parses from config today but fails
/// loudly — at *construction*, not first request — until the v2 adapter lands.
pub struct AnthropicProvider {
    _private: (),
}

impl AnthropicProvider {
    pub fn unimplemented() -> anyhow::Error {
        anyhow::anyhow!(
            "anthropic adapter not implemented yet (v2); \
             use type = \"openai_compat\" providers meanwhile"
        )
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    async fn chat_stream(
        &self,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: Option<serde_json::Value>,
        _temperature: f32,
        _reasoning_effort: Option<&str>,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        Err(Self::unimplemented())
    }
}
