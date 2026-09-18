//! Chat — the domain UI inside the stream column. `conversation` composes
//! the scrollable stream + composer + statusbar; `message`/`tool_call`/
//! `diff`/`composer`/`loading` are the per-item renderers it dispatches to.

pub mod composer;
pub mod conversation;
pub mod diff;
pub mod loading;
pub mod message;
pub mod scroll;
pub mod tool_call;
