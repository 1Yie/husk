//! The coding product's turn policy — what happens when a model round goes
//! quiet. Implements [`agent_kernel::turn_policy::TurnPolicy`], so the engine
//! loop never names this type; the host installs it at spawn
//! (`session.rs`) and hands [`coding_policy_factory`] to the subagent
//! spawner for children.
//!
//! The policy owns its per-turn state (followup counters, the architecture
//! baseline, the goal/plan controllers) behind interior mutability: the
//! engine calls [`TurnPolicy::begin_turn`] at turn start and never sees any
//! of it.

use std::path::PathBuf;
use std::sync::Arc;

use agent_kernel::tools::registry::ToolCtx;
use agent_kernel::turn_policy::{TurnPolicy, TurnReport, Verdict};

/// The coding product's contract: plan nudge, goal pushback, blueprint rework,
/// and the `computer` action cap.
///
/// Checks run in this order and the first one that asks for more work wins, so
/// a round pushed back for a plan or a goal is never also blueprint-checked.
pub struct CodingPolicy {
    /// The session's `.blueprint/` switch — shared with the host, which flips
    /// it live from the settings; the engine never sees it.
    architecture: ArchitectureSwitch,
    /// The workspace root the architecture check runs against.
    workspace_root: PathBuf,
    /// The session's goal/plan signal, as installed in the context's
    /// extension slots. `None` in contexts that carry no contract tools —
    /// the matching contract then stays silent instead of assuming.
    goal: Option<std::sync::Arc<crate::goal::GoalController>>,
    plan: Option<std::sync::Arc<crate::plan::PlanController>>,
    state: std::sync::Mutex<TurnCounters>,
}

/// The session's live `.blueprint/` on/off switch. Cloning shares the flag:
/// the host keeps one clone to flip, the policy reads the other. On by default.
#[derive(Clone)]
pub struct ArchitectureSwitch(Arc<std::sync::atomic::AtomicBool>);

impl ArchitectureSwitch {
    pub fn set(&self, enabled: bool) {
        self.0.store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn get(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for ArchitectureSwitch {
    fn default() -> Self {
        Self(Arc::new(std::sync::atomic::AtomicBool::new(true)))
    }
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
        goal: Option<std::sync::Arc<crate::goal::GoalController>>,
        plan: Option<std::sync::Arc<crate::plan::PlanController>>,
    ) -> Self {
        Self {
            architecture: ArchitectureSwitch::default(),
            workspace_root,
            goal,
            plan,
            state: std::sync::Mutex::new(TurnCounters::default()),
        }
    }

    /// Build from a context's extension slots — the same slots the goal/plan
    /// contract tools resolve their controllers through.
    pub fn from_ctx(ctx: &ToolCtx) -> Self {
        Self::new(
            ctx.workspace_root.to_path_buf(),
            ctx.ext.get::<crate::goal::GoalController>(),
            ctx.ext.get::<crate::plan::PlanController>(),
        )
    }

    /// Whether both contract controllers were found in the context. A host that
    /// assembles the coding product asserts this: `from_ctx` before
    /// [`crate::install_contract`] silently yields a policy whose plan/goal
    /// contracts never fire.
    pub fn has_contract(&self) -> bool {
        self.goal.is_some() && self.plan.is_some()
    }

    /// The handle the host flips when the user toggles `.blueprint/`.
    pub fn architecture_switch(&self) -> ArchitectureSwitch {
        self.architecture.clone()
    }

    fn plan_submitted(&self) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|p| p.state() == crate::plan::PlanState::Submitted)
    }

    fn goal_running(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(|g| g.state() == crate::goal::GoalState::Running)
    }

    fn goal_settled(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(|g| g.state() != crate::goal::GoalState::Running)
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
                Some((self.workspace_root.clone(), self.architecture.get()))
            } else {
                None
            }
        };
        if let Some((root, architecture_enabled)) = take_baseline {
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

    async fn on_quiet_round(&self, view: &dyn agent_kernel::turn_policy::TurnView, touched: Vec<PathBuf>) -> Verdict {
        let mode = view.mode();

        // Plan contract — a working turn that did tool work but never submitted
        // gets ONE nudge, so a plain question cannot be pushed twice.
        {
            let mut state = self.state.lock().unwrap();
            if mode == crate::MODE_PLAN
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
            if mode == crate::MODE_GOAL && self.goal_running() && state.goal_followups < MAX_GOAL_FOLLOWUPS {
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
            if mode == crate::MODE_GOAL && self.goal_settled() {
                if let Some(g) = &self.goal {
                    g.reset();
                }
                nudge.push_str("\n目标的完成宣告已撤回 — 修正后重新调用 `goal_complete`。");
            }
            return Verdict::Nudge { hidden: nudge };
        }
        let deliverable = if mode == crate::MODE_PLAN && self.plan_submitted() {
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
        view: &dyn agent_kernel::turn_policy::TurnView,
        touched: Vec<PathBuf>,
    ) -> Option<agent_context::blueprint::Report> {
        let root = self.workspace_root.clone();
        let enabled = self.architecture.get();
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

/// The factory the host hands to the kernel's subagent spawner: install the
/// child's own contract controllers, then build the coding policy over them.
/// The kernel calls it per delegated child — it cannot name any of this
/// itself, which is exactly the seam T9-3 opened.
pub fn coding_policy_factory(
) -> Arc<dyn Fn(&mut ToolCtx) -> Box<dyn TurnPolicy> + Send + Sync> {
    Arc::new(|ctx: &mut ToolCtx| {
        crate::install_contract(ctx);
        Box::new(CodingPolicy::from_ctx(ctx))
    })
}
