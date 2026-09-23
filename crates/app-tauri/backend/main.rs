//! `husk` — Tauri 2 desktop shell over the shared kernel wiring.
//!
//! `KernelState` (agent-kernel's `SessionManager`) owns the session actors and the
//! tagged `UiEvent` queue; `ipc/` exposes it as `agent_cmd` / `agent_session`
//! invokes plus an `agent://event` forwarder.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ipc;
mod kernel;

use kernel::KernelState;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        // System clipboard access for image paste (see `paste_clipboard`).
        .plugin(tauri_plugin_clipboard_manager::init())
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
            ipc::commands::get_ui_stats,
            ipc::commands::paste_clipboard,
            ipc::sessions::agent_session,
            ipc::sessions::mcp_probe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Husk");
}
