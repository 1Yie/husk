//! IPC layer — the `agent_*` invokes + the kernel→webview event forwarder.
//!
//! `commands` / `sessions` are the UI→kernel invokes; `forwarder` is the
//! kernel→webview pump. `main.rs` collects the commands via
//! `generate_handler![ipc::commands::agent_cmd, ipc::sessions::agent_session]`.

pub mod commands;
pub mod cue;
pub mod forwarder;
#[cfg(target_os = "linux")]
pub mod layer_shell;
#[cfg(target_os = "macos")]
pub mod macos_notifications;
pub mod overlay;
pub mod sessions;
mod sidebar;
pub mod terminal;
pub mod update;
pub mod workspace_files;
