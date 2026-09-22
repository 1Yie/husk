//! agent-ipc — the channel contract between kernel and every frontend.
//!
//! `UiCommand`/`UiEvent`/`AgentEvent` live here, not in the kernel: `agent-kernel`
//! publishes and consumes them, the frontends translate, and no frontend imports
//! kernel types for event shapes. The wire codec is serde JSON with length-prefixed
//! framing.

pub mod codec;
pub mod events;

pub use events::{AgentState, UiCommand, UiEvent};
