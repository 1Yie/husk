//! Generic conversation tools and their registry.

pub mod ask;
pub mod batch;
pub mod delegate;
pub mod builtins;
pub mod host_slots;
pub mod registry;
pub mod todo;

pub use registry::{
    Args, DesktopCtx, SandboxCtx, ToolCtx, ToolRegistry, ToolResult, ToolSpec, WorkspaceCtx,
};
