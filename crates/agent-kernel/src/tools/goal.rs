//! `goal_complete` / `goal_blocked` — the goal-mode control protocol.
//!
//! Lifecycle signals from the model to the harness, not observations or
//! mutations; registered only in the goal-mode registry. A declaration is the
//! model's claim, not harness verification: `goal_complete` ends the turn with
//! the stated result (`GoalState::Complete` is the seam for an acceptance stage),
//! `goal_blocked` ends it with the blocker as the report. `ToolError` and
//! `GoalBlocked` are different outcomes and must not be conflated.


use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// The declared lifecycle state — the model's claim, not a verified fact.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalState {
    Running = 0,
    Complete = 1,
    Blocked = 2,
}

impl GoalState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Complete,
            2 => Self::Blocked,
            _ => Self::Running,
        }
    }
}

/// The declaration payload — state alone would lose the summary/reason the
/// model attached, forcing the engine to re-parse tool text. Kept beside
/// the atomic so the engine can `take_signal()` for the final report (and
/// a future verifier gets the claim verbatim).
#[derive(Debug, Clone)]
pub enum GoalSignal {
    Complete {
        summary: String,
        reported_evidence: Option<String>,
    },
    Blocked {
        reason: String,
    },
}

/// The control channel between goal tools and the engine. Tools declare
/// through `complete()`/`blocked()` — they never touch the atomic — and
/// the engine polls `state()` / `take_signal()`. A fresh turn calls
/// `reset()` so a leftover declaration can't short-circuit it.
pub struct GoalController {
    state: AtomicU8,
    signal: Mutex<Option<GoalSignal>>,
}

impl GoalController {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(GoalState::Running as u8),
            signal: Mutex::new(None),
        }
    }

    pub fn reset(&self) {
        *self.signal.lock().unwrap() = None;
        self.state.store(GoalState::Running as u8, Ordering::Release);
    }

    pub fn complete(&self, summary: String, reported_evidence: Option<String>) {
        *self.signal.lock().unwrap() = Some(GoalSignal::Complete { summary, reported_evidence });
        self.state.store(GoalState::Complete as u8, Ordering::Release);
    }

    pub fn blocked(&self, reason: String) {
        *self.signal.lock().unwrap() = Some(GoalSignal::Blocked { reason });
        self.state.store(GoalState::Blocked as u8, Ordering::Release);
    }

    pub fn state(&self) -> GoalState {
        GoalState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn take_signal(&self) -> Option<GoalSignal> {
        self.signal.lock().unwrap().take()
    }
}

impl Default for GoalController {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GoalCompleteArgs {
    /// What was achieved — becomes the turn's final report.
    summary: String,
    /// Model-REPORTED acceptance notes: the checks you ran and their
    /// outcomes (tests passed, build green). Declared, not verified.
    #[serde(default)]
    reported_evidence: Option<String>,
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
        readonly: true, // auto-allowed control signal, not a filesystem read
        class: super::registry::ToolClass::Control,
        network: false,
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
        readonly: true, // auto-allowed control signal
        class: super::registry::ToolClass::Control,
        network: false,
        exec: Arc::new(|args, ctx| exec_blocked(args, ctx).boxed()),
    }
}

async fn exec_complete(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: GoalCompleteArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;
    // A control signal with an empty claim is meaningless — stricter than
    // ordinary tool args.
    let summary = parsed.summary.trim();
    if summary.is_empty() {
        return Err(ToolError::Args("summary must not be empty".into()));
    }
    let evidence = parsed
        .reported_evidence
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    ctx.goal.complete(summary.to_string(), evidence.clone());
    let mut out = format!("✅ 目标已宣告完成\n{summary}");
    if let Some(ev) = evidence {
        out.push_str(&format!("\n\n验收依据（模型声明，未经验证）：\n{ev}"));
    }
    Ok(ToolResult::text(out))
}

async fn exec_blocked(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: GoalBlockedArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;
    let reason = parsed.reason.trim();
    if reason.is_empty() {
        return Err(ToolError::Args("reason must not be empty".into()));
    }
    ctx.goal.blocked(reason.to_string());
    Ok(ToolResult::text(format!("⛔ 目标受阻\n{reason}")))
}
