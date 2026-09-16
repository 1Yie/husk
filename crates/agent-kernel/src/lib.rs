//! agent-kernel — pure async orchestration engine.
//!
//! Dependency law (workspace-layout.md): no Slint, no reqwest. Kernel sees
//! only normalized `StreamChunk` from agent-llm and typed events to/from
//! agent-ipc (Stage 4).
//!
//! Stage 3 scope: tool registry + phase-1 built-ins.
//! Stage 4 scope: `AgentState` machine + `SessionActor` + `channels` — the
//! ReAct loop runs headless on `MockProvider` (`cargo test -p agent-kernel`).

pub mod channels;
pub mod compaction;
pub mod engine;
pub mod permissions;
pub mod session;
pub mod tools;
