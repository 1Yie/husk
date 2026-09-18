//! `bridge` — re-exports the kernel's frontend-agnostic `SessionManager`.
//!
//! The multi-session wiring (actor threads, tagged event queue,
//! `SessionStore` persistence) lives in `agent_kernel::session_manager` so
//! both the iced shell (this crate) and the Tauri shell (`app-tauri`)
//! share one implementation. This file is kept only so existing
//! `crate::bridge::SessionManager` paths don't churn.

#[allow(unused_imports)]
pub use agent_kernel::session_manager::{SessionHandle, SessionManager};
pub use agent_kernel::session_store::RecentWorkspace;
