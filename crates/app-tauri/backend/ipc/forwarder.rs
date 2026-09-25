//! Event forwarder — drains `SessionManager::event_rx` (the tagged
//! `(workspace_root, session_id, UiEvent)` queue) and emits each as `agent://event`
//! with a `{ root, session, event }` envelope. Session ids are per-workspace, so the
//! root is what keeps a parked workspace's actors from colliding with the active one.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};

use agent_ipc::UiEvent;
use agent_kernel::session_manager::SessionManager;

pub fn spawn(app: AppHandle, rx: Receiver<(String, i64, UiEvent)>, mgr: Arc<Mutex<SessionManager>>) {
    std::thread::Builder::new()
        .name("tauri-event-fwd".into())
        .spawn(move || {
            // Tracks whether the computer-use overlay is currently up —
            // the SessionToolWhitelisted grant/revoke events flip it, and
            // a terminal AgentState tears it down. Without this, a turn
            // ending (or erroring/cancelling) left the glow + HUD parked
            // over the screen while the main window stayed hidden.
            let mut overlay_live = false;
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
                // session's `computer` whitelist so the user always sees
                // whether synthetic input is allowed.
                //
                // `run_on_main_thread` is NOT optional: this forwarder is a
                // plain `std::thread`, and Tao/GTK asserts that every window
                // create/show/close happens on the main event loop — calling
                // `WebviewWindowBuilder::build()` here SIGABRTs the process.
                if let UiEvent::SessionToolWhitelisted { tool_name, stopped } = &ev {
                    if tool_name == "computer" {
                        let stopped = *stopped;
                        overlay_live = !stopped;
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            if stopped {
                                crate::ipc::overlay::close_computer_overlay(&app2);
                            } else {
                                let _ = crate::ipc::overlay::open_computer_overlay(&app2);
                            }
                        });
                    }
                }
                // Turn end while the overlay is up — the model is done
                // driving, so revoke the whitelist, tear the glow + HUD
                // down, and bring the main window back to the front.
                if let UiEvent::StateChanged(s) = &ev {
                    let terminal = matches!(
                        s,
                        agent_ipc::AgentState::Idle
                            | agent_ipc::AgentState::Finished
                            | agent_ipc::AgentState::Failed(_)
                    );
                    if terminal && overlay_live {
                        overlay_live = false;
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            crate::ipc::overlay::end_computer_control(&app2);
                        });
                    }
                }
                // Feed the HUD a human-readable live action while a
                // `computer`/`screenshot` call is in flight. `eval` pushes
                // the text straight into the webview's DOM — no `__TAURI__`
                // listener to lose, and it lands on the main thread where
                // webview calls belong.
                if let UiEvent::ToolCallStarted { name, args_preview, .. } = &ev {
                    if name == "computer" || name == "screenshot" {
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
                    if name == "computer" || name == "screenshot" {
                        let js = crate::ipc::overlay::hud_set_status_js(
                            crate::ipc::overlay::HUD_IDLE,
                        );
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
                let _ = app.emit("agent://event", serde_json::json!({
                    "root": root,
                    "session": session_id,
                    "event": ev,
                }));
            }
        })
        .expect("spawn tauri event forwarder");
}
