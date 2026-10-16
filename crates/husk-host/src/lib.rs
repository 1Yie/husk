//! husk-host — the host layer that assembles `agent-kernel` into a running
//! product: session actors, command surface, agent modes and sandbox
//! preferences. Kernel stays a pure engine; everything with product semantics
//! lives here.

mod checkpoint;
pub mod command;
pub mod commands;
pub mod mode;
pub mod office;
pub mod plugins;
pub mod prefs;
pub mod sandbox_prefs;
pub mod session;
pub mod session_manager;
pub mod subagents;
pub mod todo_store;
