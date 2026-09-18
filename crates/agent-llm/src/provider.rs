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

    /// Whether this provider consumes structured `tool_calls` on the wire
    /// (OpenAI `function`/`function_call`, Anthropic `tool_use`).
    ///
    /// `true` (default) — the kernel keeps `ChatMessage::tool_calls`
    /// structured and the adapter serializes them natively. `false` — a
    /// text-protocol provider that only understands inline `[call: …]`
    /// echoes; the kernel's sanitize step then flattens tool_calls into
    /// message text instead. Without this branch, flattening a *native*
    /// provider's tool_calls orphans its `Role::Tool` results (P1-b).
    fn native_tool_calls(&self) -> bool {
        true
    }

    /// `tools` is a provider-agnostic JSON Schema array; the adapter maps it
    /// onto its wire shape. `temperature` is a soft hint — adapters may clamp.
    /// `reasoning_effort` is an optional hint ("low", "medium", "high", "max") for models supporting reasoning.
    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
    ) -> anyhow::Result<BoxStream<StreamChunk>>;
}
