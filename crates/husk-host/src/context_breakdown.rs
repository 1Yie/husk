//! What a request's prompt is made of — the title bar's context card.
//!
//! The provider only reports one `prompt_tokens` total. This splits the
//! request the engine is about to send into categories with the kernel's
//! compaction heuristic (ASCII ≈4 chars/token, non-ASCII ≈1 token/char,
//! ~1100/image). The UI scales the shares to the reported total, so only
//! the proportions have to be right.

use agent_llm::types::{ChatMessage, NoticeKind, Role};
use agent_llm::{BoxStream, Capabilities, LlmProvider, ModelParams, StreamChunk};
use agent_store::ContextBreakdown;
use std::sync::{Arc, Mutex};

/// UI-only extension accompanying a `Usage` event: the estimated prompt
/// composition of the round that produced it.
pub(crate) const CONTEXT_BREAKDOWN: &str = "husk:context_breakdown";

/// Request-only statistics, independent of checkpoint history/recovery. Place
/// the observer after wire transforms and before warming; replays must bypass
/// it so they cannot overwrite the latest foreground sample's composition.
#[derive(Default)]
pub(crate) struct RequestObserver {
    latest: Mutex<Option<ContextBreakdown>>,
}

impl RequestObserver {
    pub(crate) fn latest(&self) -> Option<ContextBreakdown> {
        *self.latest.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn wrap(self: &Arc<Self>, inner: Arc<dyn LlmProvider>) -> Arc<dyn LlmProvider> {
        Arc::new(ObservedProvider {
            inner,
            observer: self.clone(),
        })
    }
}

struct ObservedProvider {
    inner: Arc<dyn LlmProvider>,
    observer: Arc<RequestObserver>,
}

#[async_trait::async_trait]
impl LlmProvider for ObservedProvider {
    fn id(&self) -> &'static str {
        self.inner.id()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn native_tool_calls(&self) -> bool {
        self.inner.native_tool_calls()
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
        params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let breakdown = estimate(messages, tools.as_ref(), crate::session::is_memory_context);
        *self
            .observer
            .latest
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(breakdown);
        self.inner
            .chat_stream(
                model,
                messages,
                tools,
                temperature,
                reasoning_effort,
                params,
            )
            .await
    }
}

/// Flat vision cost per attached image — same figure the kernel budgets.
const IMAGE_TOKENS: u32 = 1100;
/// Per-message framing overhead.
const MESSAGE_OVERHEAD: u32 = 4;

/// Estimate a request's composition.
pub(crate) fn estimate(
    messages: &[ChatMessage],
    tools: Option<&serde_json::Value>,
    is_memory: impl Fn(&ChatMessage) -> bool,
) -> ContextBreakdown {
    let mut out = ContextBreakdown {
        tools: est_text(&tools.map(|t| t.to_string()).unwrap_or_default()),
        ..ContextBreakdown::default()
    };
    for m in messages {
        let text = m.content.as_deref().map(est_text).unwrap_or(0);
        let images = m.images.len() as u32 * IMAGE_TOKENS;
        let body = text + images + MESSAGE_OVERHEAD;
        match m.role {
            Role::System => out.system += body,
            Role::Tool => out.tool_results += body,
            Role::User if is_memory(m) => out.memory += body,
            Role::User if m.notice == Some(NoticeKind::Hidden) => out.other += body,
            Role::User => out.user += body,
            Role::Assistant => {
                out.assistant += body;
                out.reasoning += m.reasoning.as_deref().map(est_text).unwrap_or(0);
                out.tool_calls += m
                    .tool_calls
                    .iter()
                    .flatten()
                    .map(|c| est_text(&c.name) + est_text(&c.arguments))
                    .sum::<u32>();
            }
        }
    }
    out
}

/// ASCII chars count 1/4 token each; every non-ASCII char counts 1.
fn est_text(s: &str) -> u32 {
    let quarters: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 4 }).sum();
    u32::try_from(quarters / 4).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_llm::types::ToolCall;

    #[test]
    fn splits_a_request_by_role_and_payload() {
        let mut assistant = ChatMessage::assistant("a".repeat(40));
        assistant.reasoning = Some("r".repeat(80));
        assistant.tool_calls = Some(vec![ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: "x".repeat(36),
        }]);
        let memory = ChatMessage::user_hidden("<memory-context>\nfacts");
        let msgs = vec![
            ChatMessage::system("s".repeat(400)),
            memory.clone(),
            ChatMessage::user("你好"),
            assistant,
            ChatMessage::tool_result("call_1", "t".repeat(200)),
        ];
        let tools = serde_json::json!([{ "name": "read" }]);
        let b = estimate(&msgs, Some(&tools), |m| *m == memory);

        assert_eq!(b.system, 100 + 4);
        assert_eq!(b.user, 2 + 4);
        assert!(b.memory > 0);
        assert_eq!(b.assistant, 10 + 4);
        assert_eq!(b.reasoning, 20);
        assert_eq!(b.tool_calls, 1 + 9);
        assert_eq!(b.tool_results, 50 + 4);
        assert_eq!(b.tools, est_text(&tools.to_string()));
        assert_eq!(b.other, 0);
    }
}
