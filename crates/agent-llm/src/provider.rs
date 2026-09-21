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

/// What a wire protocol can actually express — declared by the adapter as
/// a baseline, narrowed per-deployment by `ProviderCompat` config fields.
/// The engine consults this BEFORE assembling request params, so rules
/// like "no temperature while thinking" live in one declared table instead
/// of scattering `if provider == …` guesses across adapters.
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    /// Structured tool calls on the wire (OpenAI `tool_calls`, Anthropic
    /// `tool_use`, Gemini `functionCall`). `false` = a text-protocol
    /// provider — the kernel flattens calls into message text instead.
    pub native_tool_calls: bool,
    /// Accepts a reasoning/thinking directive (`reasoning_effort`,
    /// `thinking.budget_tokens`, `thinkingConfig.thinkingBudget`). `false`
    /// → the engine never computes an effort value for this provider.
    pub reasoning: bool,
    /// `temperature` may be sent while reasoning is enabled. Anthropic's
    /// thinking mode rejects temperature outright (400); OpenAI accepts
    /// both — adapters that return `false` here drop the field themselves.
    pub temperature_with_reasoning: bool,
    /// Accepts inline image parts (`input_image` / `image` / `inlineData`).
    pub vision: bool,
}

impl Capabilities {
    /// Everything-supported baseline — adapters narrow what their wire
    /// cannot express.
    pub const fn all() -> Self {
        Self {
            native_tool_calls: true,
            reasoning: true,
            temperature_with_reasoning: true,
            vision: true,
        }
    }
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stable adapter id: `"openai_compat" | "anthropic" | "mock"`.
    fn id(&self) -> &'static str;

    /// Baseline capability table — adapters override; `ProviderCompat`
    /// config fields can narrow it further per-deployment. See field docs
    /// for what each bit gates.
    fn capabilities(&self) -> Capabilities {
        Capabilities::all()
    }

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
        self.capabilities().native_tool_calls
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
