//! Event forwarder — drains `SessionManager::event_rx` (the tagged
//! `(workspace_root, session_id, UiEvent)` queue) and emits each as `agent://event`
//! with a `{ root, session, event }` envelope. Session ids are per-workspace, so the
//! root is what keeps a parked workspace's actors from colliding with the active one.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use agent_ipc::UiEvent;
use agent_kernel::session_manager::SessionManager;

pub fn spawn(app: AppHandle, rx: Receiver<(String, i64, UiEvent)>, mgr: Arc<Mutex<SessionManager>>) {
    std::thread::Builder::new()
        .name("tauri-event-fwd".into())
        .spawn(move || {
            while let Ok((root, session_id, ev)) = rx.recv() {
                // Mirror the iced shell's `apply_session_event`: fold the
                // live sidebar flags back into the handle BEFORE emitting.
                // Nothing else writes `handle.running` on this shell, so
                // without this `sidebar_rows()` reports the creation-time
                // `false` forever and the running orb never shows.
                if let Ok(mut m) = mgr.lock() {
                    // `handle_mut_at` resolves the handle in whichever
                    // workspace owns it — a parked workspace's actor keeps
                    // streaming after a switch, and its sidebar row must
                    // keep the running flag.
                    if let Some(h) = m.handle_mut_at(&root, session_id) {
                        match &ev {
                            UiEvent::StateChanged(s) => h.running = s.is_active(),
                            UiEvent::AssistantMessage(t) => {
                                h.preview = t.chars().take(60).collect()
                            }
                            _ => {}
                        }
                    }
                }
                // Small envelope so the frontend routes by workspace +
                // session without re-parsing the payload.
                let _ = app.emit("agent://event", serde_json::json!({
                    "root": root,
                    "session": session_id,
                    "event": ev,
                }));
            }
        })
        .expect("spawn tauri event forwarder");
}
