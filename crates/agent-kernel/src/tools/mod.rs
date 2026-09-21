//! Built-in tool registry — the kernel's hands.
//!
//! Phase 1 (native-tools.md): `smart_read`, `fuzzy_patch`,
//! `smart_test_runner` plus `list_dir`, `smart_grep`, `bash`.

pub mod apply_patch;
pub mod delegate;
pub mod bash;
pub mod fs_patch;
pub mod fs_read;
pub mod goal;
pub mod grep;
pub mod list_dir;
pub mod registry;
pub mod serena;
pub mod test_runner;
pub mod todo;
pub mod util;
pub mod web_fetch;

pub use registry::{Args, ToolCtx, ToolRegistry, ToolResult, ToolSpec};
