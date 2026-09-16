//! agent-ipc — the channel contract between kernel and every frontend.
//!
//! Dependency-law pivot (workspace-layout.md): `UiCommand`/`UiEvent`/
//! `AgentEvent` live HERE, not in the kernel — `agent-kernel` publishes and
//! consumes, `app-desktop` translates, `app-cli` reuses. No frontend imports
//! kernel types for event shapes.
//!
//! Wire codec (codec.rs): serde JSON + length-prefixed framing — the same
//! envelope a future gRPC mesh reuses.

pub mod codec;
pub mod events;

pub use events::{AgentState, UiCommand, UiEvent};
