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

use agent_ipc::UiEvent;
use agent_kernel::tools::registry::ToolCtx;
use agent_kernel::turn_policy::{Effect, TurnPolicy, Verdict};
use agent_llm_core::ChatMessage;

/// The coding product's contract: plan nudge, goal pushback, blueprint rework,
/// and the `computer` action cap.
///
/// Checks run in this order and the first one that asks for more work wins, so
/// a round pushed back for a plan or a goal is never also blueprint-checked.
pub struct CodingPolicy {
    /// What this turn's tools wrote — fed by the file tools' observer, shared
    /// with the host's undo.
    writes: Option<crate::tracker::WriteTracker>,
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
            writes: None,
            architecture: ArchitectureSwitch::default(),
            workspace_root,
            goal,
            plan,
            state: std::sync::Mutex::new(TurnCounters::default()),
        }
    }

    /// Build from a context's extension slots — the same slots the goal/plan
    /// contract tools resolve their controllers through.
    pub fn from_ctx(ctx: &ToolCtx, workspace_root: PathBuf) -> Self {
        let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);
        let mut policy = Self::new(
            workspace_root,
            ctx.ext.get::<crate::goal::GoalController>(),
            ctx.ext.get::<crate::plan::PlanController>(),
        );
        policy.writes = ctx
            .ext
            .get::<crate::tracker::WriteTracker>()
            .map(|t| (*t).clone());
        policy
    }

    /// Whether both contract controllers were found in the context. A host that
    /// assembles the coding product asserts this: `from_ctx` before
    /// [`crate::install_contract`] silently yields a policy whose plan/goal
    /// contracts never fire.
    pub fn has_contract(&self) -> bool {
        self.goal.is_some() && self.plan.is_some()
    }

    /// Read "what did this turn touch" from `tracker` (the same one the host's
    /// undo and the context's write observer share). Without it the
    /// architecture check sees no touched files.
    pub fn with_write_tracker(mut self, tracker: crate::tracker::WriteTracker) -> Self {
        self.writes = Some(tracker);
        self
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

    async fn before_dispatch(&self, tool: &str, is_readonly: bool) -> Option<String> {
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

    async fn on_quiet_round(&self, view: &dyn agent_kernel::turn_policy::TurnView) -> Verdict {
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
            if mode == crate::MODE_GOAL
                && self.goal_running()
                && state.goal_followups < MAX_GOAL_FOLLOWUPS
            {
                state.goal_followups += 1;
                return Verdict::Nudge {
                    hidden:
                        "目标尚未宣告完成 — 继续推进；确实无法前进时调用 `goal_blocked` 说明阻塞。"
                            .into(),
                };
            }
        }

        // Blueprint contract — the files this turn wrote are checked against
        // `.blueprint/`. Errors buy one rework round; in goal mode a declared
        // completion is re-opened so the model has to declare again.
        let touched = self
            .writes
            .as_ref()
            .map(|w| w.touched_this_turn())
            .unwrap_or_default();
        let report = self.check_blueprint(touched).await;
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
        // The plan card lands ahead of the final message so it is the last
        // thing the composer sees; the blueprint report lands behind it.
        let before_final: Vec<Effect> = if mode == crate::MODE_PLAN && self.plan_submitted() {
            self.plan
                .as_ref()
                .and_then(|p| p.take_signal())
                .map(|card| {
                    let plan = serde_json::to_string(&card).unwrap_or_default();
                    Effect {
                        history: Some(ChatMessage::card("plan", plan.clone())),
                        ui: Some(UiEvent::Custom {
                            kind: "plan_submitted".into(),
                            payload: serde_json::json!({ "plan": plan }),
                        }),
                    }
                })
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        let after_final: Vec<Effect> = report
            .map(|r| {
                let (errors, text) = (r.errors(), r.render());
                Effect {
                    history: Some(if errors > 0 {
                        ChatMessage::notice_error(text.clone())
                    } else {
                        ChatMessage::notice(text.clone())
                    }),
                    ui: Some(UiEvent::Custom {
                        kind: "blueprint_report".into(),
                        payload: serde_json::json!({
                            "errors": errors,
                            "warnings": r.issues.len() - errors,
                            "text": text,
                        }),
                    }),
                }
            })
            .into_iter()
            .collect();
        Verdict::Finish {
            before_final,
            after_final,
        }
    }
}

impl CodingPolicy {
    /// Run the architecture check over this turn's changes. `written` is what
    /// the file tools reported; the baseline diff adds the paths a tool that
    /// bypasses the tracker (`bash`) rewrote. The baseline is taken lazily by
    /// [`TurnPolicy::before_dispatch`] before the first write, so it is the
    /// state the turn started from. A turn with no writes and no baseline
    /// skips the check.
    async fn check_blueprint(&self, written: Vec<PathBuf>) -> Option<crate::blueprint::Report> {
        let root = self.workspace_root.clone();
        let enabled = self.architecture.get();
        let baseline = self.state.lock().unwrap().architecture_baseline.clone();
        if written.is_empty() && baseline.is_none() {
            return None;
        }
        tokio::task::spawn_blocking(move || {
            let external = baseline
                .as_ref()
                .map(|before| crate::blueprint::changed_since(&root, before))
                .unwrap_or_default();
            crate::blueprint::check_turn(&root, enabled, &written, &external, baseline.as_ref())
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
    workspace_root: PathBuf,
) -> Arc<dyn Fn(&mut ToolCtx) -> Box<dyn TurnPolicy> + Send + Sync> {
    Arc::new(move |ctx: &mut ToolCtx| {
        crate::install_contract(ctx);
        // The child's own tracker, never the parent's: a child's writes must
        // not land in the parent's undo history.
        crate::tracker::WriteTracker::new(agent_context::TrackingMode::AgentOnly).install(ctx);
        Box::new(CodingPolicy::from_ctx(ctx, workspace_root.clone()))
    })
}
