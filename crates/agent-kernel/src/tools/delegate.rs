//! `delegate` — hand a scoped task to a fresh-context subagent.
//!
//! The child is a full `Engine` run in-process: its own history, its own
//! system prompt, the same provider/model/workspace/sandbox as the parent,
//! and a registry that does NOT carry `delegate` — a subagent cannot
//! delegate further (recursion guard).
//!
//! Approval in a child would deadlock — nothing answers `Ask` — so it runs
//! under a headless gate (`auto` baseline where escalations deny instead
//! of pausing). The child's cancel flag is the parent's: cancelling the
//! turn tears the delegation down with it.

use std::sync::Arc;

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolRegistry, ToolResult, ToolSpec};
use crate::engine::{Engine, EngineIo};
use crate::permissions::PermissionGate;
use agent_context::HunkTracker;
use agent_llm::types::ChatMessage;

/// The child's system prompt — intentionally small: the parent's request
/// carries the task context; the prompt only sets behavior contract.
const SUBAGENT_PROMPT: &str = "You are a delegated subagent working inside the user's workspace. \
A parent agent handed you one scoped task — complete it autonomously and return your findings or \
result as your final message. Rules: (1) no user is listening — never ask questions, report \
blockers instead; (2) stay strictly inside the task's scope; (3) verify before claiming done; \
(4) your final message is the ONLY thing the parent sees — make it self-contained.";

/// Everything `delegate` needs to spawn a child engine — built once per
/// session and carried on `ToolCtx` so the tool can clone it per call.
#[derive(Clone)]
pub struct SubagentSpawner {
    provider: Arc<dyn agent_llm::LlmProvider>,
    model: String,
    temperature: f32,
    context_window: usize,
    /// Builtins minus `delegate` — the recursion guard.
    registry_full: Arc<ToolRegistry>,
    /// `registry_full` minus non-readonly tools — `readonly: true` calls.
    registry_readonly: Arc<ToolRegistry>,
}

impl SubagentSpawner {
    pub fn new(
        provider: Arc<dyn agent_llm::LlmProvider>,
        model: impl Into<String>,
        temperature: f32,
        context_window: usize,
        registry_full: Arc<ToolRegistry>,
        registry_readonly: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            temperature,
            context_window,
            registry_full,
            registry_readonly,
        }
    }

    /// Run one child turn to completion; the result the parent sees is the
    /// child's final assistant text.
    async fn run(
        &self,
        parent_ctx: Arc<ToolCtx>,
        task: String,
        readonly: bool,
    ) -> Result<String, String> {
        // The child shares workspace + sandbox + the parent's cancel flag;
        // it does NOT share session scratch (no session attach → todo-like
        // tools degrade) and cannot delegate further (no spawner).
        let child_ctx = ToolCtx {
            workspace_root: parent_ctx.workspace_root.clone(),
            sandbox: parent_ctx.sandbox.clone(),
            session: None,
            cancel: parent_ctx.cancel.clone(),
            subagent: None,
            goal: Arc::new(crate::tools::goal::GoalController::new()),
        };
        let registry = if readonly {
            self.registry_readonly.clone()
        } else {
            self.registry_full.clone()
        };
        let mut engine = Engine::new(
            self.provider.clone(),
            registry,
            Arc::new(child_ctx),
            self.model.clone(),
            self.temperature,
        );
        engine.set_permissions(PermissionGate::for_subagent());
        engine.set_context_window(self.context_window);

        // Dead UI channel — the child's progress is not streamed to the
        // frontend in v1; try_send fails quietly on a dropped receiver.
        let (ui_tx, _ui_rx) = tokio::sync::mpsc::channel(64);
        let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel(1);
        let cancel = parent_ctx
            .cancel
            .clone()
            .unwrap_or_else(|| Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let mut io = EngineIo { ui_tx, steer_rx, cancel };
        let mut history = vec![ChatMessage::system(SUBAGENT_PROMPT)];
        let mut hunks = HunkTracker::new(agent_context::TrackingMode::AgentOnly);

        let outcome = engine
            .run_turn(&mut io, &mut history, task, &mut hunks)
            .await?;
        Ok(outcome.text)
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DelegateArgs {
    /// The complete task for the subagent — self-contained: the child sees
    /// no conversation history, only this text plus workspace state.
    task: String,
    /// Restrict the child to read-only tools (inspection, research,
    /// verification). Default false — the child may edit and run commands
    /// under the headless gate (writes auto-approve; anything that would
    /// ask a human is denied outright).
    #[serde(default)]
    readonly: bool,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "delegate",
        schema: schema_for::<DelegateArgs>(
            "Delegate one scoped task to a fresh-context subagent and return its final report.\n\
             The child gets its own engine + history (none of this conversation), the same\n\
             workspace, and no `delegate` tool of its own. Use for parallel exploration,\n\
             isolated reviews, or focused subproblems. `readonly: true` restricts the child\n\
             to inspection tools. Not available in plan mode.",
        ),
        // Not readonly: the delegation act itself writes nothing, but a
        // full child may edit — so `default` mode asks once, and `plan`
        // mode (readonly-only registry) excludes it entirely.
        readonly: false,
        exec: Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

async fn exec(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: DelegateArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;
    let task = parsed.task.trim().to_string();
    if task.is_empty() {
        return Err(ToolError::Args("`task` must be a non-empty instruction".into()));
    }
    let Some(spawner) = ctx.subagent.clone() else {
        return Err(ToolError::Failed(
            "delegation unavailable in this context".into(),
        ));
    };
    let text = spawner
        .run(ctx, task, parsed.readonly)
        .await
        .map_err(ToolError::Failed)?;
    if text.is_empty() {
        return Ok(ToolResult::text("(subagent finished with no report)"));
    }
    Ok(ToolResult::text(text))
}
