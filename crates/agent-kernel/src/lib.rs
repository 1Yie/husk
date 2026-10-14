//! agent-kernel — pure async orchestration engine.
//!
//! Dependency law (workspace-layout.md): no iced, no reqwest. Kernel sees
//! only normalized `StreamChunk` from agent-llm and typed events to/from
//! agent-ipc (Stage 4).
//!
//! Stage 3 scope: tool registry + phase-1 built-ins.
//! Stage 4 scope: `AgentState` machine + `SessionActor` + `channels` — the
//! ReAct loop runs headless on a scripted provider (`cargo test -p agent-kernel`).

pub mod blueprint;
pub mod channels;
pub mod commands;
pub mod compaction;
pub mod engine;
pub mod prefs;
pub mod hooks;
pub mod mode;
pub mod permissions;
pub mod sandbox_prefs;
pub mod session;
pub mod session_manager;
pub mod session_store;
pub mod skills;
pub mod tools;
pub mod turn_policy;

// Re-exported so a product can drive `engine::run_turn` without taking its own
// dependency on the context crate — the turn loop takes a `HunkTracker`, and
// that is the kernel's signature, not the product's concern.
pub use agent_context::HunkTracker;

pub use session_store::{
    load_appearance_settings, load_default_preferences, save_appearance_settings,
    save_default_preferences, try_load_default_preferences, AppearanceSettings, DefaultPreferences,
};
