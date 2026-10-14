//! agent-coding — the coding product's semantics, one layer above the kernel.
//!
//! Everything here names a coding-specific concept: the plan/goal/blueprint
//! turn contracts, the `blueprint` tool, and the coding turn policy. The
//! kernel stays a pure engine and knows none of them; the host (husk-host)
//! assembles this crate into the kernel's mode table, policy slot and
//! registries at spawn time.
//!
//! Dependency law: agent-coding depends on the kernel, never the reverse.

pub mod blueprint;
pub mod blueprint_tool;
pub mod goal;
pub mod plan;
pub mod policy;

use std::sync::Arc;

/// Mode ids the coding contracts are keyed on. The host's mode table
/// (`husk-host`) registers its entries under these same ids — one definition,
/// so renaming a mode cannot silently disarm the policy that watches it.
pub const MODE_PLAN: &str = "plan";
pub const MODE_GOAL: &str = "goal";

use agent_kernel::tools::registry::ToolCtx;

/// Install the goal/plan contract controllers this context owns — its own
/// pair, never shared: a derived ctx (batch, `with_cancel`) reads the same
/// pair, while a delegated child builds its own so its completion signal
/// never lands on the parent's controller. Hosts call this once per root
/// context at assembly; [`policy::coding_policy_factory`] calls it per
/// subagent child.
pub fn install_contract(ctx: &mut ToolCtx) {
    ctx.ext.insert(Arc::new(goal::GoalController::new()));
    ctx.ext.insert(Arc::new(plan::PlanController::new()));
}
