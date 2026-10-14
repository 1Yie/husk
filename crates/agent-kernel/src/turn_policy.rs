//! Turn policy seam: what happens when a model round goes quiet.
//!
//! The turn loop in `engine.rs` owns sampling, tool dispatch and history. The
//! product rules that push a quiet turn back to work (plan submission, goal
//! declaration, architecture rework) live behind [`TurnPolicy`], so the loop
//! does not name them — the coding product's rule set is `agent-coding`'s
//! `CodingPolicy`, installed by the host.
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
/// session installs the coding product's policy (`agent-coding`'s
/// `CodingPolicy`) at spawn instead.
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
