//! `LlmProvider` trait + `BoxStream` alias.
//!
//! `Send + Sync` + `Arc` — the kernel swaps providers by replacing the `Arc`,
//! which is what makes hot-switching free (applies next turn, never
//! interrupts an in-flight stream).

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::types::{ChatMessage, StreamChunk};

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = anyhow::Result<T>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stable adapter id: `"openai_compat" | "anthropic" | "mock"`.
    fn id(&self) -> &'static str;

    /// `tools` is a provider-agnostic JSON Schema array; the adapter maps it
    /// onto its wire shape. `temperature` is a soft hint — adapters may clamp.
    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
    ) -> anyhow::Result<BoxStream<StreamChunk>>;
}
