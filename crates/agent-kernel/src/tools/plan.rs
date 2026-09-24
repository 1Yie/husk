//! `submit_plan` — the plan-mode deliverable contract.
//!
//! A lifecycle signal from the model to the harness, mirroring the goal
//! contract: the model declares its plan through `submit()` instead of
//! trailing off with prose. The controller carries the structured payload
//! beside the atomic state so the engine can `take_signal()` at the quiet
//! point — the card the UI renders is the submitted data verbatim, never a
//! re-parse of assistant text.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use serde::{Deserialize, Serialize};

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

/// Whether the model has declared its plan this turn.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanState {
    Running = 0,
    Submitted = 1,
}

impl PlanState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Submitted,
            _ => Self::Running,
        }
    }
}

/// One step of the plan — a unit of work the executor walks top to bottom.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanStep {
    /// Short imperative title — the step list's headline.
    pub title: String,
    /// What the step changes and why — enough to execute without re-asking.
    #[serde(default)]
    pub detail: String,
    /// Repo-relative paths the step touches.
    #[serde(default)]
    pub files: Vec<String>,
}

/// The submitted deliverable — `Serialize` so the engine can hand it to the
/// UI as a JSON payload without re-deriving the shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// One-paragraph outcome the plan achieves — the card's summary line.
    pub summary: String,
    /// Ordered implementation steps.
    pub steps: Vec<PlanStep>,
    /// How the result is proven — commands to run, checks to eyeball.
    #[serde(default)]
    pub verification: Vec<String>,
    /// Known risks and open questions the approver should weigh.
    #[serde(default)]
    pub risks: Vec<String>,
}

/// The control channel between `submit_plan` and the engine — same shape as
/// `GoalController`: tools declare, the engine polls at the quiet point.
pub struct PlanController {
    state: AtomicU8,
    signal: Mutex<Option<Plan>>,
}

impl PlanController {
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(PlanState::Running as u8),
            signal: Mutex::new(None),
        }
    }

    pub fn reset(&self) {
        *self.signal.lock().unwrap() = None;
        self.state.store(PlanState::Running as u8, Ordering::Release);
    }

    pub fn submit(&self, plan: Plan) {
        *self.signal.lock().unwrap() = Some(plan);
        self.state.store(PlanState::Submitted as u8, Ordering::Release);
    }

    pub fn state(&self) -> PlanState {
        PlanState::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn take_signal(&self) -> Option<Plan> {
        self.signal.lock().unwrap().take()
    }
}

impl Default for PlanController {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubmitPlanArgs {
    /// One-paragraph outcome the plan achieves.
    summary: String,
    /// Ordered implementation steps — each with a title, what it changes,
    /// and the repo-relative files it touches.
    steps: Vec<PlanStep>,
    /// How the result is proven — commands to run, checks to eyeball.
    #[serde(default)]
    verification: Vec<String>,
    /// Known risks and open questions the approver should weigh.
    #[serde(default)]
    risks: Vec<String>,
}

pub fn spec_submit() -> ToolSpec {
    ToolSpec {
        name: "submit_plan",
        schema: schema_for::<SubmitPlanArgs>(
            "Submit the implementation plan — the plan-mode deliverable. Call ONCE when the \
             analysis is complete: summary, ordered steps (title/detail/files), verification, \
             risks. The user reviews this card and decides whether to execute it in build mode.",
        ),
        readonly: true, // control signal — declares, never mutates
        class: super::registry::ToolClass::Control,
        network: false,
        exec: Arc::new(|args, ctx| exec_submit(args, ctx).boxed()),
    }
}

async fn exec_submit(args: Args, ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: SubmitPlanArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;
    // A plan with no steps is a status report, not a deliverable.
    if parsed.summary.trim().is_empty() {
        return Err(ToolError::Args("summary must not be empty".into()));
    }
    if parsed.steps.is_empty() {
        return Err(ToolError::Args("steps must not be empty".into()));
    }
    let plan = Plan {
        summary: parsed.summary.trim().to_string(),
        steps: parsed
            .steps
            .into_iter()
            .filter(|s| !s.title.trim().is_empty())
            .collect(),
        verification: parsed.verification,
        risks: parsed.risks,
    };
    let step_count = plan.steps.len();
    ctx.plan.submit(plan);
    Ok(ToolResult::text(format!(
        "📋 计划已提交（{step_count} 步）— 等待用户审阅。"
    )))
}
