//! Event forwarder — drains `SessionManager::event_rx` (the tagged
//! `(workspace_root, session_id, UiEvent)` queue) and emits each as `agent://event`
//! with a `{ root, session, event }` envelope. Session ids are per-workspace, so the
//! root is what keeps a parked workspace's actors from colliding with the active one.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};

use agent_ipc::UiEvent;
use husk_host::session_manager::SessionManager;

pub fn spawn(
    app: AppHandle,
    rx: Receiver<(String, i64, UiEvent)>,
    mgr: Arc<Mutex<SessionManager>>,
) {
    std::thread::Builder::new()
        .name("tauri-event-fwd".into())
        .spawn(move || {
            // The overlay is bound to the session that was granted `computer`
            // (`OverlayState`), not to "whatever is active": another session's
            // events, grants and turn ends never touch it.
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
                // The computer-use overlay lives outside the React tree —
                // spawn/despawn the two windows in lockstep with the
                // session's `computer` grant so the user always sees
                // whether synthetic input is allowed.
                //
                // `run_on_main_thread` is NOT optional: this forwarder is a
                // plain `std::thread`, and Tao/GTK asserts that every window
                // create/show/close happens on the main event loop — calling
                // `WebviewWindowBuilder::build()` here SIGABRTs the process.
                let controlling = |app: &AppHandle| {
                    app.state::<crate::ipc::overlay::OverlayState>()
                        .0
                        .lock()
                        .map(|g| g.as_ref().is_some_and(|c| c.is(&root, session_id)))
                        .unwrap_or(false)
                };
                if let UiEvent::SessionToolWhitelisted { tool_name, stopped } = &ev {
                    if tool_name == "computer" && !*stopped {
                        // Grant: this session now holds the desktop. Capture its
                        // stop handles while the manager is at hand.
                        let held = mgr.lock().ok().and_then(|m| {
                            m.handle_at(&root, session_id)
                                .map(|h| crate::ipc::overlay::Controlled::of(&root, session_id, h))
                        });
                        if let Some(c) = held {
                            if let Ok(mut g) = app.state::<crate::ipc::overlay::OverlayState>().0.lock() {
                                *g = Some(c);
                            }
                            let app2 = app.clone();
                            let _ = app.run_on_main_thread(move || {
                                let _ = crate::ipc::overlay::open_computer_overlay(&app2);
                            });
                        }
                    } else if tool_name == "computer" && controlling(&app) {
                        // Revoke on the controlling session (the kernel ends the
                        // grant at every turn end): release the overlay. A revoke
                        // on any OTHER session is not ours to act on.
                        if let Ok(mut g) = app.state::<crate::ipc::overlay::OverlayState>().0.lock() {
                            *g = None;
                        }
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            crate::ipc::overlay::close_and_restore(&app2);
                        });
                    }
                }
                // Backstop: the controlling session reached a terminal state but
                // its revoke event never arrived (a dropped event must not leave
                // the glow + HUD parked over the screen).
                if let UiEvent::StateChanged(s) = &ev {
                    let terminal = matches!(
                        s,
                        agent_ipc::AgentState::Idle
                            | agent_ipc::AgentState::Finished
                            | agent_ipc::AgentState::Failed(_)
                    );
                    if terminal && controlling(&app) {
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            crate::ipc::overlay::stop_controlled(&app2);
                        });
                    }
                }
                // Feed the HUD a human-readable live action while the
                // controlling session's `computer`/`screenshot` call is in
                // flight. `eval` pushes the text straight into the webview's
                // DOM — no `__TAURI__` listener to lose, and it lands on the
                // main thread where webview calls belong.
                if let UiEvent::ToolCallStarted {
                    name, args_preview, ..
                } = &ev
                {
                    if (name == "computer" || name == "screenshot") && controlling(&app) {
                        let js = crate::ipc::overlay::hud_set_status_js(
                            &crate::ipc::overlay::hud_action_label(name, args_preview),
                        );
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            if let Some(w) = app2.get_webview_window("computer-hud") {
                                let _ = w.eval(js.as_str());
                            }
                        });
                    }
                } else if let UiEvent::ToolCallFinished { name, .. } = &ev {
                    if (name == "computer" || name == "screenshot") && controlling(&app) {
                        let js =
                            crate::ipc::overlay::hud_set_status_js(crate::ipc::overlay::HUD_IDLE);
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            if let Some(w) = app2.get_webview_window("computer-hud") {
                                let _ = w.eval(js.as_str());
                            }
                        });
                    }
                }
                // Small envelope so the frontend routes by workspace +
                // session without re-parsing the payload.
                let _ = app.emit(
                    "agent://event",
                    serde_json::json!({
                        "root": root,
                        "session": session_id,
                        "event": ev,
                    }),
                );
            }
        })
        .expect("spawn tauri event forwarder");
}
