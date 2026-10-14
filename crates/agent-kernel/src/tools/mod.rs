//! Built-in tool registry.
//!
//! `smart_read`, `fuzzy_patch`, `smart_test_runner`, `list_dir`, `smart_grep`,
//! `bash` and the session/skill tools.

pub mod ask;
pub mod batch;
pub mod delegate;
// File tools live in `agent-tools-fs`: generic over the context there, so
// this crate depends on them rather than the other way round.
pub use agent_tools_fs::{apply_patch, fs_patch, fs_read, grep, list_dir};
pub use agent_tools_computer::{computer, screenshot};
pub use agent_tools_shell::{bash, sandbox_cfg, services, test_runner, util};
pub use agent_tools_web::{assets, web_fetch};
#[cfg(feature = "tree-sitter")]
pub use agent_tools_fs::outline_ts;

pub mod builtins;
pub mod host_slots;
pub mod registry;
pub mod todo;

pub use registry::{
    Args, DesktopCtx, SandboxCtx, ToolCtx, ToolRegistry, ToolResult, ToolSpec, WorkspaceCtx,
};
