//! IPC layer — the `agent_*` invokes + the kernel→webview event forwarder.
//!
//! `commands` / `sessions` are the UI→kernel invokes; `forwarder` is the
//! kernel→webview pump. `main.rs` collects the commands via
//! `generate_handler![ipc::commands::agent_cmd, ipc::sessions::agent_session]`.

pub mod commands;
pub mod forwarder;
pub mod overlay;
pub mod sessions;
