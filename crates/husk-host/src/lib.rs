//! husk-host — the host layer that assembles `agent-kernel` into a running
//! product: session actors, command surface, agent modes and sandbox
//! preferences. Kernel stays a pure engine; everything with product semantics
//! lives here.

mod checkpoint;
pub mod child_route;
pub mod command;
pub mod commands;
mod context_breakdown;
pub mod links;
pub mod link_policy;
pub mod mode;
pub mod office;
pub mod plugins;
pub mod prefs;
pub mod repeat_guard;
pub mod rules;
pub mod sandbox_prefs;
pub mod session;
pub mod session_manager;
pub mod subagents;
pub mod todo_store;
