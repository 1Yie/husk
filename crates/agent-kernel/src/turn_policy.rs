//! Turn policy seam: what happens when a model round goes quiet.
//!
//! The turn loop in `engine.rs` owns sampling, tool dispatch and history. The
//! product rules that push a quiet turn back to work (plan submission, goal
//! declaration, blueprint rework) live behind [`TurnPolicy`], so the loop does
//! not name them. [`CodingPolicy`] is the coding product's rule set.

use std::path::PathBuf;

/// Per-turn counters and baselines a policy keeps across quiet rounds.
#[derive(Default)]
pub struct TurnState {
    /// Goal-mode pushbacks spent this turn.
    pub goal_followups: usize,
    /// Plan-mode nudges spent this turn.
    pub plan_followups: usize,
    /// Blueprint rework rounds spent this turn.
    pub blueprint_followups: usize,
    /// Workspace snapshot taken before the first write, if any.
    pub blueprint_baseline: Option<crate::blueprint::Baseline>,
    /// Whether the baseline was attempted, so a read-only turn pays for it at
    /// most once and never at all.
    pub blueprint_baseline_taken: bool,
}

/// The policy's answer when the model finished a round without tool calls.
pub enum Verdict {
    /// The turn may end. `report` is the blueprint result to show after the
    /// final answer, if a check ran.
    Finish {
        report: Option<agent_context::blueprint::Report>,
    },
    /// The turn must continue. The engine records the quiet round's text, then
    /// pushes `hidden` as the nudge the model sees next.
    Nudge { hidden: String },
}

/// What a policy may read and reset about the turn, without holding the engine.
pub trait TurnView: Send + Sync {
    /// Whether the session is in plan mode.
    fn in_plan_mode(&self) -> bool;
    /// Whether the session is in goal mode.
    fn in_goal_mode(&self) -> bool;
    /// Whether this turn ran at least one tool call.
    fn did_tool_work(&self) -> bool;
    /// Whether the model submitted a plan this turn. The engine keeps the plan
    /// itself and lands it as the deliverable after the final text.
    fn plan_submitted(&self) -> bool;
    /// Whether the goal is still running (neither complete nor blocked).
    fn goal_running(&self) -> bool;
    /// Whether the goal is no longer running (declared complete or blocked).
    fn goal_settled(&self) -> bool;
    /// Withdraw the goal declaration so the model must declare again.
    fn reset_goal(&self);
    /// Workspace root the blueprint check runs against.
    fn workspace_root(&self) -> PathBuf;
    /// Whether `.blueprint/` checking is enabled for this session.
    fn blueprint_enabled(&self) -> bool;
}

/// Product rules applied to a turn that went quiet.
#[async_trait::async_trait]
pub trait TurnPolicy: Send + Sync {
    /// Decide whether a quiet round may end the turn. `touched` lists the files
    /// this turn changed.
    async fn on_quiet_round(
        &self,
        state: &mut TurnState,
        view: &dyn TurnView,
        touched: Vec<PathBuf>,
    ) -> Verdict;
}

/// The coding product's contract: plan nudge, goal pushback, blueprint rework.
///
/// Checks run in this order and the first one that asks for more work wins, so
/// a round pushed back for a plan or a goal is never also blueprint-checked.
pub struct CodingPolicy;

/// Nudge texts and caps. They are the product's contract with the model.
const MAX_GOAL_FOLLOWUPS: usize = 8;
const MAX_PLAN_FOLLOWUPS: usize = 1;
const MAX_BLUEPRINT_FOLLOWUPS: usize = 1;

#[async_trait::async_trait]
impl TurnPolicy for CodingPolicy {
    async fn on_quiet_round(
        &self,
        state: &mut TurnState,
        view: &dyn TurnView,
        touched: Vec<PathBuf>,
    ) -> Verdict {
        // Plan contract — a working turn that did tool work but never submitted
        // gets ONE nudge, so a plain question cannot be pushed twice.
        if view.in_plan_mode()
            && !view.plan_submitted()
            && view.did_tool_work()
            && state.plan_followups < MAX_PLAN_FOLLOWUPS
        {
            state.plan_followups += 1;
            return Verdict::Nudge {
                hidden: "若方案已成型，请调用 `submit_plan` 提交结构化计划（summary/steps/verification/risks）；若本轮只是回答问题、没有方案要交付，直接结束即可。".into(),
            };
        }

        // Goal contract — in goal mode going quiet undeclared is not a finish.
        if view.in_goal_mode()
            && view.goal_running()
            && state.goal_followups < MAX_GOAL_FOLLOWUPS
        {
            state.goal_followups += 1;
            return Verdict::Nudge {
                hidden: "目标尚未宣告完成 — 继续推进；确实无法前进时调用 `goal_blocked` 说明阻塞。".into(),
            };
        }

        // Blueprint contract — the files this turn wrote are checked against
        // `.blueprint/`. Errors buy one rework round; in goal mode a declared
        // completion is re-opened so the model has to declare again.
        let report = self.check_blueprint(state, view, touched).await;
        if let Some(report) = report
            .as_ref()
            .filter(|r| r.errors() > 0 && state.blueprint_followups < MAX_BLUEPRINT_FOLLOWUPS)
        {
            state.blueprint_followups += 1;
            let mut nudge = crate::blueprint::nudge(report);
            if view.in_goal_mode() && view.goal_settled() {
                view.reset_goal();
                nudge.push_str("\n目标的完成宣告已撤回 — 修正后重新调用 `goal_complete`。");
            }
            return Verdict::Nudge { hidden: nudge };
        }
        Verdict::Finish { report }
    }
}

impl CodingPolicy {
    /// Run the blueprint check over this turn's files. The baseline is taken
    /// lazily by the engine before the first write, so it is the state the turn
    /// started from. A turn with no touched files and no baseline skips the check.
    async fn check_blueprint(
        &self,
        state: &mut TurnState,
        view: &dyn TurnView,
        touched: Vec<PathBuf>,
    ) -> Option<agent_context::blueprint::Report> {
        let root = view.workspace_root();
        let enabled = view.blueprint_enabled();
        let mut touched = touched;
        let baseline = state.blueprint_baseline.clone();
        if touched.is_empty() && baseline.is_none() {
            return None;
        }
        tokio::task::spawn_blocking(move || {
            if let Some(before) = &baseline {
                for p in crate::blueprint::changed_since(&root, before) {
                    if !touched.contains(&p) {
                        touched.push(p);
                    }
                }
            }
            crate::blueprint::check_turn(&root, enabled, &touched, baseline.as_ref())
        })
        .await
        .ok()
        .flatten()
    }
}

/// The engine's view of one quiet round, built when the round goes quiet.
///
/// It holds clones of the engine's shared handles, not a borrow of the engine,
/// so the loop can keep its `&mut self` while the policy runs.
pub struct EngineTurnView {
    pub goal: std::sync::Arc<crate::tools::goal::GoalController>,
    pub plan: std::sync::Arc<crate::tools::plan::PlanController>,
    pub mode: std::sync::Arc<std::sync::RwLock<crate::mode::AgentMode>>,
    pub workspace_root: PathBuf,
    pub blueprint_enabled: bool,
    /// Tool calls run so far this turn (the loop's `tool_calls_run`).
    pub tool_calls_run: usize,
}

impl EngineTurnView {
    fn mode(&self) -> crate::mode::AgentMode {
        self.mode.read().map(|m| *m).unwrap_or_default()
    }
}

impl TurnView for EngineTurnView {
    fn in_plan_mode(&self) -> bool {
        self.mode() == crate::mode::AgentMode::Plan
    }
    fn in_goal_mode(&self) -> bool {
        self.mode() == crate::mode::AgentMode::Goal
    }
    fn did_tool_work(&self) -> bool {
        self.tool_calls_run > 0
    }
    fn plan_submitted(&self) -> bool {
        self.plan.state() == crate::tools::plan::PlanState::Submitted
    }
    fn goal_running(&self) -> bool {
        self.goal.state() == crate::tools::goal::GoalState::Running
    }
    fn goal_settled(&self) -> bool {
        self.goal.state() != crate::tools::goal::GoalState::Running
    }
    fn reset_goal(&self) {
        self.goal.reset();
    }
    fn workspace_root(&self) -> PathBuf {
        self.workspace_root.clone()
    }
    fn blueprint_enabled(&self) -> bool {
        self.blueprint_enabled
    }
}
