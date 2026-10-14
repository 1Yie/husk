//! Turn policy seam: what happens when a model round goes quiet.
//!
//! The turn loop in `engine.rs` owns sampling, tool dispatch and history. The
//! product rules that push a quiet turn back to work (plan submission, goal
//! declaration, architecture rework) live behind [`TurnPolicy`], so the loop
//! does not name them. [`CodingPolicy`] is the coding product's rule set.
//!
//! The policy owns its per-turn state (followup counters, the architecture
//! baseline, the goal/plan controllers) behind interior mutability: the engine
//! calls [`TurnPolicy::begin_turn`] at turn start and never sees any of it.
//! What crosses the seam is generic — the current mode id, whether tools ran,
//! and an opaque report/payload pair the engine renders without knowing what
//! produced them.

use std::path::PathBuf;

/// The policy's answer when the model finished a round without tool calls.
pub enum Verdict {
    /// The turn may end. `report` is a turn-end check to render after the
    /// final answer, if one ran; `deliverable` is a serialized payload the
    /// engine lands as the turn's deliverable (the plan card), if the policy
    /// holds one.
    Finish {
        report: Option<TurnReport>,
        deliverable: Option<String>,
    },
    /// The turn must continue. The engine records the quiet round's text, then
    /// pushes `hidden` as the nudge the model sees next.
    Nudge { hidden: String },
}

/// A turn-end check result in the shape the engine renders — the policy
/// produces it, the engine never looks inside beyond counts and text.
pub struct TurnReport {
    pub errors: usize,
    pub warnings: usize,
    pub text: String,
}

/// What a policy may read about the turn at its quiet point, without holding
/// the engine. Mode ids are the host's ([`crate::engine::ModeEntry`]); the
/// policy compares them against the ids its own contract tools were built for.
pub trait TurnView: Send + Sync {
    /// The mode currently in the shared slot.
    fn mode(&self) -> String;
    /// Whether this turn ran at least one tool call.
    fn did_tool_work(&self) -> bool;
    /// Whether the session's architecture-checking switch is on.
    fn architecture_enabled(&self) -> bool;
}

/// Product rules applied to a turn that went quiet.
#[async_trait::async_trait]
pub trait TurnPolicy: Send + Sync {
    /// Called when a turn starts. A policy keeps per-turn counters and
    /// snapshots behind interior mutability; this is where it clears them —
    /// including any lifecycle controllers it holds, so a leftover
    /// `goal_complete`/`submit_plan` from an earlier turn cannot short-circuit
    /// this one (the contracts are per-turn).
    fn begin_turn(&self);

    /// Called for each tool call that cleared permission and is about to run.
    /// `is_readonly` keys the architecture baseline (taken lazily before the
    /// first write); `architecture_enabled` is the session's live switch for
    /// it. Returns `Some(message)` to refuse the call: the engine reports the
    /// message as the call's failed result and does not dispatch.
    async fn before_dispatch(
        &self,
        tool: &str,
        is_readonly: bool,
        architecture_enabled: bool,
    ) -> Option<String>;

    /// Decide whether a quiet round may end the turn. `touched` lists the
    /// files this turn changed.
    async fn on_quiet_round(&self, view: &dyn TurnView, touched: Vec<PathBuf>) -> Verdict;
}

/// The default policy: every quiet round finishes, nothing is refused.
///
/// A second product assembled straight on the kernel gets this; a coding
/// session installs [`CodingPolicy`] at spawn instead.
pub struct NoopPolicy;

#[async_trait::async_trait]
impl TurnPolicy for NoopPolicy {
    fn begin_turn(&self) {}
    async fn before_dispatch(
        &self,
        _tool: &str,
        _is_readonly: bool,
        _architecture_enabled: bool,
    ) -> Option<String> {
        None
    }
    async fn on_quiet_round(&self, _view: &dyn TurnView, _touched: Vec<PathBuf>) -> Verdict {
        Verdict::Finish {
            report: None,
            deliverable: None,
        }
    }
}

/// The coding product's contract: plan nudge, goal pushback, blueprint rework,
/// and the `computer` action cap.
///
/// Checks run in this order and the first one that asks for more work wins, so
/// a round pushed back for a plan or a goal is never also blueprint-checked.
pub struct CodingPolicy {
    /// The workspace root the architecture check runs against.
    workspace_root: PathBuf,
    /// The session's goal/plan signal, as installed in the context's
    /// extension slots. `None` in contexts that carry no contract tools —
    /// the matching contract then stays silent instead of assuming.
    goal: Option<std::sync::Arc<crate::tools::goal::GoalController>>,
    plan: Option<std::sync::Arc<crate::tools::plan::PlanController>>,
    state: std::sync::Mutex<TurnCounters>,
}

/// Nudge texts and caps. They are the product's contract with the model.
const MAX_GOAL_FOLLOWUPS: usize = 8;
const MAX_PLAN_FOLLOWUPS: usize = 1;
const MAX_BLUEPRINT_FOLLOWUPS: usize = 1;

/// Hard cap on `computer` calls in one turn. A session grant removes the
/// per-call prompt, so this is what bounds a runaway or injected loop of
/// synthetic input; the model is told to stop and report instead.
const MAX_COMPUTER_ACTIONS_PER_TURN: usize = 60;

#[derive(Default)]
struct TurnCounters {
    goal_followups: usize,
    plan_followups: usize,
    blueprint_followups: usize,
    /// Workspace snapshot taken before the first write, if any.
    architecture_baseline: Option<crate::blueprint::Baseline>,
    /// Whether the baseline was attempted, so a read-only turn pays for it at
    /// most once and never at all.
    architecture_baseline_taken: bool,
    computer_actions: usize,
}

impl CodingPolicy {
    pub fn new(
        workspace_root: PathBuf,
        goal: Option<std::sync::Arc<crate::tools::goal::GoalController>>,
        plan: Option<std::sync::Arc<crate::tools::plan::PlanController>>,
    ) -> Self {
        Self {
            workspace_root,
            goal,
            plan,
            state: std::sync::Mutex::new(TurnCounters::default()),
        }
    }

    /// Build from a context's extension slots — the same slots the goal/plan
    /// contract tools resolve their controllers through.
    pub fn from_ctx(ctx: &crate::tools::registry::ToolCtx) -> Self {
        Self::new(
            ctx.workspace_root.to_path_buf(),
            ctx.ext.get::<crate::tools::goal::GoalController>(),
            ctx.ext.get::<crate::tools::plan::PlanController>(),
        )
    }

    fn plan_submitted(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|p| p.state() == crate::tools::plan::PlanState::Submitted)
    }

    fn goal_running(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(|g| g.state() == crate::tools::goal::GoalState::Running)
    }

    fn goal_settled(&self) -> bool {
        self.goal.as_ref().is_some_and(|g| {
            g.state() != crate::tools::goal::GoalState::Running
        })
    }
}

#[async_trait::async_trait]
impl TurnPolicy for CodingPolicy {
    fn begin_turn(&self) {
        *self.state.lock().unwrap() = TurnCounters::default();
        if let Some(g) = &self.goal {
            g.reset();
        }
        if let Some(p) = &self.plan {
            p.reset();
        }
    }

    async fn before_dispatch(
        &self,
        tool: &str,
        is_readonly: bool,
        architecture_enabled: bool,
    ) -> Option<String> {
        // The architecture baseline is the workspace as the turn found it,
        // taken lazily right before the first tool that can change it — before
        // any write lands, so it is the state the turn started from. A
        // read-only turn never pays for it. The flag is claimed under the
        // lock; the (slow) snapshot runs without holding it — a std
        // MutexGuard is not Send, and this future must stay Send.
        let take_baseline = {
            let mut state = self.state.lock().unwrap();
            if tool == "computer" {
                state.computer_actions += 1;
                if state.computer_actions > MAX_COMPUTER_ACTIONS_PER_TURN {
                    return Some(format!(
                        "computer action limit reached ({MAX_COMPUTER_ACTIONS_PER_TURN} per turn) — \
                         stop driving the desktop and report progress to the user"
                    ));
                }
            }
            if !state.architecture_baseline_taken && !is_readonly {
                state.architecture_baseline_taken = true;
                Some(self.workspace_root.clone())
            } else {
                None
            }
        };
        if let Some(root) = take_baseline {
            let baseline = tokio::task::spawn_blocking(move || {
                crate::blueprint::snapshot(&root, architecture_enabled)
            })
            .await
            .ok()
            .flatten();
            self.state.lock().unwrap().architecture_baseline = baseline;
        }
        None
    }

    async fn on_quiet_round(&self, view: &dyn TurnView, touched: Vec<PathBuf>) -> Verdict {
        let mode = view.mode();

        // Plan contract — a working turn that did tool work but never submitted
        // gets ONE nudge, so a plain question cannot be pushed twice.
        {
            let mut state = self.state.lock().unwrap();
            if mode == "plan"
                && view.did_tool_work()
                && !self.plan_submitted()
                && state.plan_followups < MAX_PLAN_FOLLOWUPS
            {
                state.plan_followups += 1;
                return Verdict::Nudge {
                    hidden: "若方案已成型，请调用 `submit_plan` 提交结构化计划（summary/steps/verification/risks）；若本轮只是回答问题、没有方案要交付，直接结束即可。".into(),
                };
            }

            // Goal contract — in goal mode going quiet undeclared is not a finish.
            if mode == "goal" && self.goal_running() && state.goal_followups < MAX_GOAL_FOLLOWUPS {
                state.goal_followups += 1;
                return Verdict::Nudge {
                    hidden: "目标尚未宣告完成 — 继续推进；确实无法前进时调用 `goal_blocked` 说明阻塞。".into(),
                };
            }
        }

        // Blueprint contract — the files this turn wrote are checked against
        // `.blueprint/`. Errors buy one rework round; in goal mode a declared
        // completion is re-opened so the model has to declare again.
        let report = self.check_blueprint(view, touched).await;
        let mut state = self.state.lock().unwrap();
        if let Some(r) = report
            .as_ref()
            .filter(|r| r.errors() > 0 && state.blueprint_followups < MAX_BLUEPRINT_FOLLOWUPS)
        {
            state.blueprint_followups += 1;
            let mut nudge = crate::blueprint::nudge(r);
            if mode == "goal" && self.goal_settled() {
                if let Some(g) = &self.goal {
                    g.reset();
                }
                nudge.push_str("\n目标的完成宣告已撤回 — 修正后重新调用 `goal_complete`。");
            }
            return Verdict::Nudge { hidden: nudge };
        }
        let deliverable = if mode == "plan" && self.plan_submitted() {
            self.plan.as_ref().and_then(|p| {
                p.take_signal()
                    .as_ref()
                    .map(|card| serde_json::to_string(card).unwrap_or_default())
            })
        } else {
            None
        };
        Verdict::Finish {
            report: report.map(|r| TurnReport {
                errors: r.errors(),
                warnings: r.issues.len() - r.errors(),
                text: r.render(),
            }),
            deliverable,
        }
    }
}

impl CodingPolicy {
    /// Run the architecture check over this turn's files. The baseline is
    /// taken lazily by [`TurnPolicy::before_dispatch`] before the first write,
    /// so it is the state the turn started from. A turn with no touched files
    /// and no baseline skips the check.
    async fn check_blueprint(
        &self,
        view: &dyn TurnView,
        touched: Vec<PathBuf>,
    ) -> Option<agent_context::blueprint::Report> {
        let root = self.workspace_root.clone();
        let enabled = view.architecture_enabled();
        let mut touched = touched;
        let baseline = self.state.lock().unwrap().architecture_baseline.clone();
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
