//! `hooks.rs` — ordered `AgentHook` chain over the state machine.
//!
//! Hooks run in registration order under a 2s timeout: a slow hook degrades to
//! `Continue` instead of stalling the turn. `before_tool_execute` runs ahead of
//! the permission gate, so a hook can veto or rewrite args but never approve.
//!
//! The chain is **shared** (`Arc` inside, `Clone` outside): `SessionManager`
//! builds it from the loaded plugins' declarations, hands a clone to every
//! session's engine, and `reload_plugins` swaps the contents in place — so a
//! hook added in settings applies to live sessions on their next turn.
//! [`CommandAgentHook`] is the adapter for a plugin's declared local command;
//! built-ins implement [`AgentHook`] directly.

use std::sync::Arc;
use std::time::Duration;

use agent_ipc::AgentState;
use agent_llm::types::{ChatMessage, ToolCall};
use async_trait::async_trait;

/// Per-hook latency cap — a slow hook degrades to `Continue`.
pub const HOOK_TIMEOUT: Duration = Duration::from_secs(2);

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

    /// Before a tool executes — `Err(reason)` vetoes, `Ok(())` proceeds to the
    /// permission gate (a hook can't approve).
    async fn before_tool_execute(&self, _call: &mut ToolCall) -> anyhow::Result<()> {
        Ok(())
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

    /// The turn's final answer, mutable before it lands in history and on
    /// screen — `on_response` hooks rewrite `text` in place.
    async fn on_response(&self, _text: &mut String) {}
}

/// The ordered chain — `run_*` helpers apply the timeout + degrade policy.
///
/// Cheap to clone; every clone shares one hook list and one transition slot,
/// which is what makes a mid-session reload and a de-duplicated transition
/// stream possible (the engine announces a state the session then mirrors —
/// that is one transition, not two).
#[derive(Clone)]
pub struct HookChain {
    inner: Arc<Inner>,
}

struct Inner {
    hooks: std::sync::RwLock<Vec<Arc<dyn AgentHook>>>,
    /// Last state any clone of this chain observed. Seeded `Idle` so the
    /// first real transition (→ `ScanningWorkspace`) reports its origin.
    last_state: std::sync::Mutex<AgentState>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            hooks: std::sync::RwLock::new(Vec::new()),
            last_state: std::sync::Mutex::new(AgentState::Idle),
        }
    }
}

impl Default for HookChain {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner::default()),
        }
    }
}

impl HookChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, hook: Arc<dyn AgentHook>) {
        self.write().push(hook);
    }

    /// Swap the whole chain — the reload path. `&self`, so the sessions and
    /// engines already holding a clone see the new hooks immediately.
    pub fn replace(&self, hooks: Vec<Arc<dyn AgentHook>>) {
        let mut guard = self
            .inner
            .hooks
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = hooks;
    }

    pub fn is_empty(&self) -> bool {
        self.snapshot().is_empty()
    }

    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    /// Hook ids, in chain order — the settings/status read path.
    pub fn ids(&self) -> Vec<String> {
        self.snapshot().iter().map(|h| h.id().to_string()).collect()
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Vec<Arc<dyn AgentHook>>> {
        self.inner
            .hooks
            .write()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Clone the hook list out of the lock — the lock is never held across an
    /// `await` (a hook is free to be slow; the timeout is what bounds it).
    fn snapshot(&self) -> Vec<Arc<dyn AgentHook>> {
        self.inner
            .hooks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Record `new` as the current state; returns the state it replaced, or
    /// `None` when nothing changed. Clones share one slot, so a state
    /// announced twice (engine + session) reports once.
    fn take_transition(&self, new: &AgentState) -> Option<AgentState> {
        let mut last = self
            .inner
            .last_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *last == *new {
            return None;
        }
        Some(std::mem::replace(&mut *last, new.clone()))
    }

    /// `on_state_transition` chain — awaited inline.
    pub async fn run_state_transition(&self, old: &AgentState, new: &AgentState) {
        for h in self.snapshot() {
            if tokio::time::timeout(HOOK_TIMEOUT, h.on_state_transition(old, new))
                .await
                .is_err()
            {
                tracing::warn!(hook = h.id(), "on_state_transition timed out");
            }
        }
    }

    /// Observe a state the kernel just entered, awaiting the hooks.
    pub async fn observe(&self, new: &AgentState) {
        let Some(old) = self.take_transition(new) else {
            return;
        };
        if self.is_empty() {
            return;
        }
        self.run_state_transition(&old, new).await;
    }

    /// Observe a state without waiting — for the sync emit sites (`Engine::
    /// set_state` on the turn's hot path). A hook never delays the state the
    /// UI is about to render; outside a runtime the dispatch is skipped
    /// rather than blocking on it.
    pub fn observe_nowait(&self, new: &AgentState) {
        let Some(old) = self.take_transition(new) else {
            return;
        };
        if self.is_empty() {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let chain = self.clone();
        let new = new.clone();
        handle.spawn(async move { chain.run_state_transition(&old, &new).await });
    }

    /// `on_user_input` chain — returns the first non-`Continue` action or
    /// `Continue` when every hook passes.
    pub async fn run_on_user_input(&self, input: &str) -> HookAction {
        for h in self.snapshot() {
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

    /// `before_tool_execute` chain — any veto stops the tool, carrying the
    /// vetoing hook's reason to the model (empty → a generic line).
    pub async fn run_before_tool(&self, call: &mut ToolCall) -> Result<(), String> {
        for h in self.snapshot() {
            match tokio::time::timeout(HOOK_TIMEOUT, h.before_tool_execute(call)).await {
                Ok(Ok(())) => continue,
                Ok(Err(e)) => {
                    // A veto is a decision, not a failure — its text IS the
                    // reason the model sees, so an empty one still names the
                    // hook that refused.
                    let reason = e.to_string();
                    let reason = reason.trim();
                    return Err(if reason.is_empty() {
                        format!("tool `{}` vetoed by hook {}", call.name, h.id())
                    } else {
                        reason.to_string()
                    });
                }
                _ => continue, // error/timeout → degrade to allow
            }
        }
        Ok(())
    }

    /// `after_tool_execute` chain — hooks may mutate `output`.
    pub async fn run_after_tool(&self, call: &ToolCall, output: &mut String) {
        for h in self.snapshot() {
            let _ = tokio::time::timeout(HOOK_TIMEOUT, h.after_tool_execute(call, output)).await;
        }
    }

    /// `on_response` chain — hooks may rewrite the final answer in place.
    pub async fn run_on_response(&self, text: &mut String) {
        for h in self.snapshot() {
            let _ = tokio::time::timeout(HOOK_TIMEOUT, h.on_response(text)).await;
        }
    }
}

/// A plugin's declared hook, adapted to the in-process trait.
///
/// The wire contract (payloads, verdicts, degrade rules) lives in
/// `agent_plugin::hooks`; this is the only place the kernel knows a hook may
/// be an external command.
pub struct CommandAgentHook {
    /// `<plugin>:<event>` — the chain's identity for this hook.
    name: String,
    spec: agent_plugin::CommandHook,
}

impl CommandAgentHook {
    pub fn new(spec: agent_plugin::CommandHook) -> Self {
        Self {
            name: spec.id(),
            spec,
        }
    }
}

#[async_trait]
impl AgentHook for CommandAgentHook {
    fn id(&self) -> &str {
        &self.name
    }

    async fn on_user_input(&self, input: &str) -> anyhow::Result<HookAction> {
        // One declaration = one event: without this guard a `before_tool`
        // hook would be spawned on every user input (and could answer with a
        // verdict the input path misreads).
        if self.spec.event != "on_user_input" {
            return Ok(HookAction::Continue);
        }
        Ok(match self.spec.run_input(input).await {
            agent_plugin::InputVerdict::Continue => HookAction::Continue,
            agent_plugin::InputVerdict::Block(r) => HookAction::BlockTurn(r),
            agent_plugin::InputVerdict::Inject(n) => HookAction::InjectSystemNote(n),
        })
    }

    async fn before_tool_execute(&self, call: &mut ToolCall) -> anyhow::Result<()> {
        if self.spec.event != "before_tool_execute" {
            return Ok(());
        }
        if !self.spec.matches(&call.name) {
            return Ok(());
        }
        // `ToolCall.arguments` is the wire's incremental JSON text, not a
        // value — a rewrite round-trips through `serde_json`.
        let args: serde_json::Value = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({}));
        match self.spec.run_before_tool(&call.name, &args).await {
            agent_plugin::ToolVerdict::Continue => Ok(()),
            // A veto travels as the trait's `Err` — its text is the reason.
            agent_plugin::ToolVerdict::Veto(reason) => Err(anyhow::anyhow!(reason)),
            agent_plugin::ToolVerdict::Rewrite(new_args) => {
                call.arguments = serde_json::to_string(&new_args)?;
                Ok(())
            }
        }
    }

    async fn after_tool_execute(&self, call: &ToolCall, output: &mut String) -> anyhow::Result<()> {
        if self.spec.event != "after_tool_execute" {
            return Ok(());
        }
        if !self.spec.matches(&call.name) {
            return Ok(());
        }
        let args: serde_json::Value = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({}));
        if let Some(rewritten) = self.spec.run_after_tool(&call.name, &args, output).await {
            *output = rewritten;
        }
        Ok(())
    }

    async fn on_state_transition(&self, old: &AgentState, new: &AgentState) {
        if self.spec.event != "on_state_transition" {
            return;
        }
        if !self.spec.matches(&state_key(new)) {
            return;
        }
        self.spec
            .run_transition(&state_key(old), &state_key(new))
            .await;
    }

    async fn on_response(&self, text: &mut String) {
        if self.spec.event != "on_response" {
            return;
        }
        if let Some(rewritten) = self.spec.run_response(text).await {
            *text = rewritten;
        }
    }
}

/// How a state is named to a hook filter/payload — the variant name, without
/// the payload (`Failed("…")` → `Failed`, so a filter can actually match it).
fn state_key(state: &AgentState) -> String {
    match state {
        AgentState::Idle => "Idle",
        AgentState::ScanningWorkspace => "ScanningWorkspace",
        AgentState::Reasoning => "Reasoning",
        AgentState::StreamingToken => "StreamingToken",
        AgentState::AwaitingToolConfirmation { .. } => "AwaitingToolConfirmation",
        AgentState::AwaitingPluginConsent { .. } => "AwaitingPluginConsent",
        AgentState::AwaitingConsent { .. } => "AwaitingConsent",
        AgentState::Branching { .. } => "Branching",
        AgentState::ExecutingTool { .. } => "ExecutingTool",
        AgentState::Compacting => "Compacting",
        AgentState::Finished => "Finished",
        AgentState::Failed(_) => "Failed",
    }
    .to_string()
}

/// A plugin set's declared hooks as chain entries, in declaration order.
pub fn manifest_hooks(specs: Vec<agent_plugin::CommandHook>) -> Vec<Arc<dyn AgentHook>> {
    specs
        .into_iter()
        .map(|s| Arc::new(CommandAgentHook::new(s)) as Arc<dyn AgentHook>)
        .collect()
}

/// Build a chain from a plugin set — the boot path's convenience shape.
pub fn chain_from_specs(specs: Vec<agent_plugin::CommandHook>) -> HookChain {
    let chain = HookChain::new();
    chain.replace(manifest_hooks(specs));
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Count(AtomicUsize);

    #[async_trait]
    impl AgentHook for Count {
        fn id(&self) -> &str {
            "count"
        }
        async fn on_state_transition(&self, _o: &AgentState, _n: &AgentState) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A state announced by both the engine and the session's own mirror is
    /// ONE transition — clones share the slot.
    #[tokio::test]
    async fn transitions_fire_once_per_real_change() {
        let counter = Arc::new(Count(AtomicUsize::new(0)));
        let chain = HookChain::new();
        chain.replace(vec![counter.clone()]);

        chain.observe(&AgentState::ScanningWorkspace).await;
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        // Same state again (the session mirroring the engine) — no event.
        chain.observe(&AgentState::ScanningWorkspace).await;
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        // A real change fires, and a clone sees the same timeline.
        let clone = chain.clone();
        clone.observe(&AgentState::Reasoning).await;
        assert_eq!(counter.0.load(Ordering::SeqCst), 2);
        chain
            .observe(&AgentState::ExecutingTool {
                tool_name: "bash".into(),
            })
            .await;
        assert_eq!(counter.0.load(Ordering::SeqCst), 3);
    }

    /// The sync emit path fires too, and never blocks the caller.
    #[tokio::test]
    async fn observe_nowait_dispatches() {
        let counter = Arc::new(Count(AtomicUsize::new(0)));
        let chain = HookChain::new();
        chain.replace(vec![counter.clone()]);
        chain.observe_nowait(&AgentState::Reasoning);
        // The dispatch is spawned; yield so it runs.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    }

    /// A chain with no hooks costs a compare — the default state of the app.
    #[tokio::test]
    async fn empty_chain_is_inert() {
        let chain = HookChain::new();
        assert!(chain.is_empty());
        chain.observe(&AgentState::Reasoning).await;
        chain.observe_nowait(&AgentState::Finished);
        // No hook ran, and the slot still tracked the change.
        assert_eq!(chain.take_transition(&AgentState::Finished), None);
    }

    /// A veto carries the hook's reason; a bare veto still names the hook.
    #[tokio::test]
    async fn veto_reasons_reach_the_caller() {
        struct Veto(&'static str);
        #[async_trait]
        impl AgentHook for Veto {
            fn id(&self) -> &str {
                "veto"
            }
            async fn before_tool_execute(&self, _c: &mut ToolCall) -> anyhow::Result<()> {
                Err(anyhow::anyhow!(self.0))
            }
        }

        let chain = HookChain::new();
        chain.replace(vec![Arc::new(Veto("no push today"))]);
        let mut call = ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        };
        assert_eq!(
            chain.run_before_tool(&mut call).await.unwrap_err(),
            "no push today"
        );

        let chain = HookChain::new();
        chain.replace(vec![Arc::new(Veto(""))]);
        let err = chain.run_before_tool(&mut call).await.unwrap_err();
        assert!(err.contains("veto"), "{err}");
    }
}
