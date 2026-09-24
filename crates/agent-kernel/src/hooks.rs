//! `hooks.rs` — ordered `AgentHook` chain over the state machine.
//!
//! Hooks run in registration order under a 2s timeout: a slow hook degrades to
//! `Continue` instead of stalling the turn. `before_tool_execute` runs ahead of
//! the permission gate, so a hook can veto or rewrite args but never approve.
//! The chain ships empty until a concrete need appears.

use std::sync::Arc;
use std::time::Duration;

use agent_ipc::AgentState;
use agent_llm::types::{ChatMessage, ToolCall};
use async_trait::async_trait;

/// Per-hook latency cap — a slow hook degrades to `Continue`.
const HOOK_TIMEOUT: Duration = Duration::from_secs(2);

/// What a hook decided.
#[derive(Debug)]
pub enum HookAction {
    /// No-op — the turn proceeds untouched.
    Continue,
    /// Rewrite the outbound context before sampling.
    MutateMessages(Vec<ChatMessage>),
    /// Stop this turn with a reason (surfaced as a system message).
    BlockTurn(String),
    /// Silent addition to the system prompt.
    InjectSystemNote(String),
}

#[async_trait]
pub trait AgentHook: Send + Sync {
    fn id(&self) -> &str;

    /// Before the user input becomes a turn.
    async fn on_user_input(&self, _input: &str) -> anyhow::Result<HookAction> {
        Ok(HookAction::Continue)
    }

    /// Before a tool executes — `false` vetoes, `true` proceeds to the
    /// permission gate (a hook can't approve).
    async fn before_tool_execute(&self, _call: &mut ToolCall) -> anyhow::Result<bool> {
        Ok(true)
    }

    /// After a tool executes — may mutate `output` (truncation applies).
    async fn after_tool_execute(
        &self,
        _call: &ToolCall,
        _output: &mut String,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// On every `AgentState` transition.
    async fn on_state_transition(&self, _old: &AgentState, _new: &AgentState) {}
}

/// The ordered chain — `run_*` helpers apply the timeout + degrade policy.
#[derive(Default)]
pub struct HookChain {
    hooks: Vec<Arc<dyn AgentHook>>,
}

impl HookChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, hook: Arc<dyn AgentHook>) {
        self.hooks.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// `on_user_input` chain — returns the first non-`Continue` action or
    /// `Continue` when every hook passes.
    pub async fn run_on_user_input(&self, input: &str) -> HookAction {
        for h in &self.hooks {
            match tokio::time::timeout(HOOK_TIMEOUT, h.on_user_input(input)).await {
                Ok(Ok(HookAction::Continue)) => continue,
                Ok(Ok(action)) => return action,
                Ok(Err(e)) => {
                    tracing::warn!(hook = h.id(), "on_user_input failed: {e}");
                    continue;
                }
                Err(_) => {
                    tracing::warn!(hook = h.id(), "on_user_input timed out — Continue");
                    continue;
                }
            }
        }
        HookAction::Continue
    }

    /// `before_tool_execute` chain — any veto (`false`) stops the tool.
    pub async fn run_before_tool(&self, call: &mut ToolCall) -> bool {
        for h in &self.hooks {
            match tokio::time::timeout(HOOK_TIMEOUT, h.before_tool_execute(call)).await {
                Ok(Ok(true)) => continue,
                Ok(Ok(false)) => return false,
                _ => continue, // error/timeout → degrade to allow
            }
        }
        true
    }

    /// `after_tool_execute` chain — hooks may mutate `output`.
    pub async fn run_after_tool(&self, call: &ToolCall, output: &mut String) {
        for h in &self.hooks {
            let _ = tokio::time::timeout(HOOK_TIMEOUT, h.after_tool_execute(call, output)).await;
        }
    }

    /// `on_state_transition` — fire-and-forget, no timeout wait (it's `()`
    /// and sync-quick by contract).
    pub async fn run_state_transition(&self, old: &AgentState, new: &AgentState) {
        for h in &self.hooks {
            h.on_state_transition(old, new).await;
        }
    }
}
