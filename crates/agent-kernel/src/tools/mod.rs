//! Built-in tool registry.
//!
//! `smart_read`, `fuzzy_patch`, `smart_test_runner`, `list_dir`, `smart_grep`,
//! `bash` and the session/skill tools.

pub mod apply_patch;
pub mod ask;
pub mod bash;
pub mod batch;
pub mod computer;
pub mod delegate;
pub mod fs_patch;
pub mod fs_read;
pub mod goal;
pub mod grep;
pub mod list_dir;
pub(crate) mod net;
#[cfg(feature = "tree-sitter")]
pub mod outline_ts;
pub mod plan;
pub mod registry;
pub mod remember;
pub(crate) mod sandbox_cfg;
pub mod screenshot;
pub mod serena;
pub mod skill;
pub mod test_runner;
pub mod todo;
pub mod util;
pub mod web_fetch;

pub use registry::{Args, ToolCtx, ToolRegistry, ToolResult, ToolSpec};
