//! Adapters between Husk's plugin runtime and the generic kernel contracts.

use std::sync::{Arc, RwLock};

use agent_ipc::AgentState;
use agent_kernel::hooks::{AgentHook, HookAction, HookChain};
use agent_kernel::plugins::{PluginHandle, PluginRouter};
use agent_kernel::tools::ToolResult;
use agent_llm::types::ToolCall;
use async_trait::async_trait;

/// The runtime handle used by discovery, management and reload operations.
pub type RuntimePluginHandle = Arc<RwLock<Option<Arc<agent_plugin::PluginManager>>>>;

struct RuntimeRouter(RuntimePluginHandle);

impl RuntimeRouter {
    fn snapshot(&self) -> Option<Arc<agent_plugin::PluginManager>> {
        self.0.read().ok().and_then(|g| g.clone())
    }
}

#[async_trait]
impl PluginRouter for RuntimeRouter {
    fn exported_tools(&self) -> Vec<serde_json::Value> {
        self.snapshot()
            .map(|m| m.exported_tools())
            .unwrap_or_default()
    }

    async fn dispatch_tool_call(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Option<Result<ToolResult, String>> {
        let manager = self.snapshot()?;
        match manager.dispatch_tool_call(name, args).await {
            Err(error) if error.starts_with("no plugin owns tool") => None,
            result => Some(result.map(ToolResult::text)),
        }
    }
}

/// A session keeps this adapter while reload swaps the runtime behind it.
pub fn router_handle(runtime: RuntimePluginHandle) -> PluginHandle {
    let router: Arc<dyn PluginRouter> = Arc::new(RuntimeRouter(runtime));
    Arc::new(RwLock::new(Some(router)))
}

/// A plugin's declared hook, adapted to the in-process trait.
///
/// The wire contract (payloads, verdicts, degrade rules) lives in
/// `agent_plugin::hooks`; Husk adapts these commands to the generic kernel hook chain.
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
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or_else(|_| serde_json::json!({}));
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
        let args: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or_else(|_| serde_json::json!({}));
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
