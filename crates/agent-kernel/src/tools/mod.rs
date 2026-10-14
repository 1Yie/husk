//! Built-in tool registry.
//!
//! `smart_read`, `fuzzy_patch`, `smart_test_runner`, `list_dir`, `smart_grep`,
//! `bash` and the session/skill tools.

pub mod ask;
pub mod bash;
pub mod batch;
pub mod blueprint;
pub mod computer;
pub mod delegate;
pub mod goal;
pub mod office;
// File tools live in `agent-tools-fs`: generic over the context there, so
// this crate depends on them rather than the other way round.
pub use agent_tools_fs::{apply_patch, fs_patch, fs_read, grep, list_dir};
pub use agent_tools_web::{assets, web_fetch};
#[cfg(feature = "tree-sitter")]
pub use agent_tools_fs::outline_ts;

pub mod builtins;
pub mod host_slots;
pub mod plan;
pub mod registry;
pub mod remember;
pub(crate) mod sandbox_cfg;
pub mod screenshot;
pub mod serena;
pub mod services;
pub mod skill;
pub mod test_runner;
pub mod todo;
pub mod util;

pub use registry::{Args, ToolCtx, ToolRegistry, ToolResult, ToolSpec};
