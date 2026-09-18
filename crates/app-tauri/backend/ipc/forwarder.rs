//! Event forwarder — drains `SessionManager::event_rx` (the tagged
//! `(session_id, UiEvent)` queue) and emits each as `agent://event`.
//! One thread for the app lifetime; the webview listener routes by
//! session id from the `{ session, event }` envelope.

use std::sync::mpsc::Receiver;

use tauri::{AppHandle, Emitter};

use agent_ipc::UiEvent;

pub fn spawn(app: AppHandle, rx: Receiver<(i64, UiEvent)>) {
    std::thread::Builder::new()
        .name("tauri-event-fwd".into())
        .spawn(move || {
            while let Ok((session_id, ev)) = rx.recv() {
                // Small envelope so the frontend routes by session without
                // re-parsing the payload.
                let _ = app.emit("agent://event", serde_json::json!({
                    "session": session_id,
                    "event": ev,
                }));
            }
        })
        .expect("spawn tauri event forwarder");
}
