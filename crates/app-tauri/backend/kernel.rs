//! Kernel handle — the `SessionManager` held in Tauri state.
//!
//! `SessionManager` isn't `Sync`; Tauri state wants `Send`, so it's wrapped
//! in a `Mutex` here. Session actors keep their own threads — this state
//! only touches the command / steer / decision / cancel handles.

use std::sync::{Arc, Mutex};

use agent_kernel::session_manager::SessionManager;

/// The host's own directories — the product names itself here, and nowhere
/// below the frontend does.
///
/// Also carries the `agent-rs` → `husk` rename: a one-time `fs::rename` moves
/// the whole legacy config/data tree over when the new dir is absent, so an
/// existing install keeps its providers, plugins and sessions.
pub fn app_paths() -> agent_paths::AppPaths {
    let paths = agent_paths::AppPaths::from_app_name("husk", ".husk")
        .unwrap_or_else(|| agent_paths::AppPaths::new("", "", ".husk"));
    if let Some(base) = dirs::config_dir() {
        migrate_legacy_dir(&base.join("agent-rs"), &paths.config_dir);
    }
    if let Some(base) = dirs::data_dir() {
        migrate_legacy_dir(&base.join("agent-rs"), &paths.data_dir);
    }
    paths
}

fn migrate_legacy_dir(legacy: &std::path::Path, dir: &std::path::Path) {
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(legacy, dir);
    }
}

/// The shared kernel state — one `SessionManager` for the app lifetime.
/// `Arc` so the event forwarder can share it: it needs `handle_mut` to
/// keep sidebar `running`/`preview` truthful as events stream past.
pub struct KernelState(pub Arc<Mutex<SessionManager>>);

impl KernelState {
    /// Boot the manager (resumes the latest session or creates one) and
    /// return it together with its `event_rx` — the receiver is moved out
    /// to the forwarder, the manager itself stays behind the mutex.
    /// Each event is tagged `(workspace_root, session_id)` — session ids
    /// are per-workspace, the root keeps a parked workspace's still-running
    /// actors from colliding with the active one's.
    pub fn spawn() -> (
        Self,
        std::sync::mpsc::Receiver<(String, i64, agent_ipc::UiEvent)>,
    ) {
        let (mut mgr, rx) = SessionManager::spawn(app_paths());
        if std::env::args().any(|arg| arg == "--new-window") && mgr.workspace_active {
            mgr.new_session();
        }
        (Self(Arc::new(Mutex::new(mgr))), rx)
    }
}
