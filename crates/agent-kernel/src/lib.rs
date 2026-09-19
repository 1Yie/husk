//! agent-kernel — pure async orchestration engine.
//!
//! Dependency law (workspace-layout.md): no iced, no reqwest. Kernel sees
//! only normalized `StreamChunk` from agent-llm and typed events to/from
//! agent-ipc (Stage 4).
//!
//! Stage 3 scope: tool registry + phase-1 built-ins.
//! Stage 4 scope: `AgentState` machine + `SessionActor` + `channels` — the
//! ReAct loop runs headless on `MockProvider` (`cargo test -p agent-kernel`).

pub mod channels;
pub mod commands;
pub mod compaction;
pub mod engine;
pub mod hooks;
pub mod permissions;
pub mod session;
pub mod session_store;
pub mod session_manager;
pub mod steering;
pub mod tools;

pub use session_store::{
    load_default_preferences, save_default_preferences, try_load_default_preferences,
    DefaultPreferences,
};
