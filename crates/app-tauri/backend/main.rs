//! `husk` — Tauri 2 desktop shell over the shared kernel wiring.
//!
//! Rust side is deliberately thin: `KernelState` (agent-kernel's
//! `SessionManager`) owns the session actors + tagged `UiEvent` queue;
//! `ipc/` exposes that as `agent_cmd` / `agent_session` invokes and an
//! `agent://event` forwarder. No business logic here.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ipc;
mod kernel;

use kernel::KernelState;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Boot the shared kernel manager, take its event receiver, and
            // forward it to the webview. `event_rx` is `std::sync::mpsc`
            // so it's moved, not cloned — the manager lives in `KernelState`.
            let (state, rx) = KernelState::spawn();
            let mgr = state.0.clone();
            app.manage(state);
            ipc::forwarder::spawn(app.handle().clone(), rx, mgr);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::commands::agent_cmd,
            ipc::sessions::agent_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Husk");
}
