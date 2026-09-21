//! `goal_complete` / `goal_blocked` — the goal-mode contract tools.
//!
//! Registered only in the goal-mode registry: the model declares the
//! outcome through one of these instead of just stopping. Both write the
//! shared `ctx.goal` signal the engine polls when the model goes quiet —
//! 0 = still running, 1 = complete, 2 = blocked. `goal_blocked` doesn't
//! fail the turn; it ends it with the reason as the final report.

use std::sync::Arc;

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GoalCompleteArgs {
    /// What was achieved — becomes the turn's final report.
    summary: String,
    /// Verified acceptance evidence: the checks that ran and their
    /// outcomes (tests passed, build green, files changed). Plain text.
    #[serde(default)]
    evidence: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GoalBlockedArgs {
    /// The concrete blocker — the missing decision, broken dependency, or
    /// external action required. Becomes the turn's final report.
    reason: String,
}

pub fn spec_complete() -> ToolSpec {
    ToolSpec {
        name: "goal_complete",
        schema: schema_for::<GoalCompleteArgs>(
            "Declare the goal achieved — ends the turn. Only call this when the goal is \
             verifiably met; a declared completion IS the result the user sees.",
        ),
        readonly: true, // a signal write to session scratch, not the workspace
        exec: Arc::new(|args, ctx| exec_complete(args, ctx).boxed()),
    }
}

pub fn spec_blocked() -> ToolSpec {
    ToolSpec {
        name: "goal_blocked",
        schema: schema_for::<GoalBlockedArgs>(
            "Declare the goal unreachable — ends the turn with the blocker as the report. \
             Call when you cannot make progress without outside input.",
        ),
        readonly: true,
        exec: Arc::new(|args, ctx| exec_blocked(args, ctx).boxed()),
    }
}

async fn exec_complete(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: GoalCompleteArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;
    ctx.goal.store(1, std::sync::atomic::Ordering::Relaxed);
    let mut out = format!("✅ 目标已宣告完成\n{}", parsed.summary.trim());
    if let Some(ev) = parsed.evidence.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()) {
        out.push_str(&format!("\n\n验收依据：\n{ev}"));
    }
    Ok(ToolResult::text(out))
}

async fn exec_blocked(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: GoalBlockedArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;
    ctx.goal.store(2, std::sync::atomic::Ordering::Relaxed);
    Ok(ToolResult::text(format!(
        "⛔ 目标受阻\n{}",
        parsed.reason.trim()
    )))
}
