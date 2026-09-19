//! Kernel handle — the `SessionManager` held in Tauri state.
//!
//! `SessionManager` isn't `Sync`; Tauri state wants `Send`, so it's wrapped
//! in a `Mutex` here. Session actors keep their own threads — this state
//! only touches the command / steer / decision / cancel handles.

use std::sync::{Arc, Mutex};

use agent_kernel::session_manager::SessionManager;

/// The shared kernel state — one `SessionManager` for the app lifetime.
/// `Arc` so the event forwarder can share it: it needs `handle_mut` to
/// keep sidebar `running`/`preview` truthful as events stream past.
pub struct KernelState(pub Arc<Mutex<SessionManager>>);

impl KernelState {
    /// Boot the manager (resumes the latest session or creates one) and
    /// return it together with its `event_rx` — the receiver is moved out
    /// to the forwarder, the manager itself stays behind the mutex.
    pub fn spawn() -> (Self, std::sync::mpsc::Receiver<(i64, agent_ipc::UiEvent)>) {
        let (mgr, rx) = SessionManager::spawn();
        (Self(Arc::new(Mutex::new(mgr))), rx)
    }
}
