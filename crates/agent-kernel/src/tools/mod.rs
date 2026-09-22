//! Built-in tool registry.
//!
//! `smart_read`, `fuzzy_patch`, `smart_test_runner`, `list_dir`, `smart_grep`,
//! `bash` and the session/skill tools.

pub mod apply_patch;
pub mod ask;
pub mod batch;
pub mod delegate;
pub mod bash;
pub mod fs_patch;
pub mod fs_read;
pub mod goal;
pub mod grep;
pub mod list_dir;
pub(crate) mod net;
pub mod registry;
pub mod serena;
pub mod skill;
pub mod test_runner;
pub mod todo;
pub mod util;
pub mod web_fetch;

pub use registry::{Args, ToolCtx, ToolRegistry, ToolResult, ToolSpec};
