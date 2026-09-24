//! `agent_cmd` — the single invoke for all `UiCommand`s.
//!
//! One command, the enum carries the intent. `Cancel`, `Steer` and `ToolDecision`
//! bypass the command pump (direct flag / channel) so they are not queued behind a
//! running turn.

use agent_ipc::UiCommand;
use tauri::State;

use crate::kernel::KernelState;

/// `paste_clipboard` — read the *system* clipboard from Rust, for the composer's
/// Ctrl+V and its context menu.
///
/// WebKitGTK's own `paste` event is not a dependable carrier for images: pasting
/// into a plain `<textarea>` can deliver no `clipboardData` at all. The bytes are
/// reachable here regardless.
///
/// `kind` = `"image"` (default) stages the clipboard image as an attachment;
/// `"text"` returns the clipboard string. An image-less clipboard is `null`, not an
/// error — the caller then leaves the browser's own paste alone.
///
/// `async` so `read_image` does not run on the main thread (arboard can deadlock on
/// Linux when the clipboard holds data this app copied).
#[tauri::command]
pub async fn paste_clipboard(
    app: tauri::AppHandle,
    state: State<'_, KernelState>,
    kind: String,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let clipboard = app.clipboard();

    if kind == "text" {
        return Ok(match clipboard.read_text() {
            Ok(text) if !text.is_empty() => serde_json::json!({ "kind": "text", "text": text }),
            _ => serde_json::Value::Null,
        });
    }

    let Ok(image) = clipboard.read_image() else {
        return Ok(serde_json::Value::Null);
    };
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Ok(serde_json::Value::Null);
    }
    let png = crate::ipc::sessions::encode_png(image.rgba(), width, height)?;
    // Lock only long enough to copy the root — never across the clipboard or
    // file work above.
    let root = {
        let mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.workspace_root.clone()
    };
    if root.as_os_str().is_empty() {
        return Err("没有打开的工作区".into());
    }
    crate::ipc::sessions::stage_clipboard_image(&png, &root)
}

/// `get_ui_stats` — delivery counters for every live session's UI event
/// queue. The status chip reads them so a lagging consumer (dropped deltas,
/// deepening queue) is visible instead of silent. Control events are never
/// dropped, so `dropped > 0` always means *text* was lost.
#[tauri::command]
pub fn get_ui_stats(
    state: State<'_, KernelState>,
) -> Result<agent_kernel::channels::UiStatsSnapshot, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(mgr.ui_stats())
}

#[tauri::command]
pub fn agent_cmd(state: State<'_, KernelState>, cmd: UiCommand) -> Result<(), String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
        let (cancel, steer_tx, decision, ask, permissions, agent_mode, ui, cmd_tx) = {
        let Some(handle) = mgr.active() else {
            return Err("no active session".into());
        };
        (
            handle.cancel.clone(),
            handle.steer_tx.clone(),
            handle.decision.clone(),
            handle.ask.clone(),
            handle.permissions.clone(),
                handle.agent_mode.clone(),
            handle.ui.clone(),
            handle.cmd_tx.clone(),
        )
    };

    // `Cancel` bypasses the command pump (direct flag) so it isn't queued
    // behind a running `run_turn`.
    if matches!(cmd, UiCommand::Cancel) {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(());
    }
    if let UiCommand::Steer { text } = &cmd {
        steer_tx.try_send(text.clone()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let UiCommand::ToolDecision { request_id, approved } = &cmd {
        *decision.lock().map_err(|e| e.to_string())? = Some((*request_id, *approved));
        return Ok(());
    }
    if let UiCommand::AnswerQuestion { request_id, answer } = &cmd {
        // Only a matched answer resolves the parked card — `answer()` returns
        // false for a stale/duplicate request_id, and echoing that would drop
        // a *different* live question's card.
        if ask.answer(*request_id, answer.clone()) {
            // Echo the resolution so the frontend clears `pendingQuestion`
            // from the view buffer — the answered card must not resurrect on
            // the next session switch.
            let _ = ui.send(agent_ipc::UiEvent::QuestionAnswered {
                request_id: *request_id,
            });
        }
        return Ok(());
    }
    // Composer settings are PER-SESSION: the command goes only to the active
    // actor, which applies it to its own engine and persists it into its own
    // SessionMeta. They deliberately do NOT touch the manager-level workspace
    // default (`mgr.set_*`/`persist_prefs`) — that's what used to leak one
    // session's model/mode/level into every other session's display & spawn.
    // `SetPermissionMode`/`SetAgentMode` still write the live handle slots here
    // so a mid-turn swap applies at the next dispatch rather than after the turn.
    if let UiCommand::SetPermissionMode { mode } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            *gate = agent_kernel::permissions::PermissionGate::from_mode_str(mode);
        }
    }
    if let UiCommand::SetAgentMode { mode } = &cmd {
        if let Ok(mut slot) = agent_mode.write() {
            *slot = agent_kernel::mode::AgentMode::from_str(mode);
        }
    }
    cmd_tx.try_send(cmd).map_err(|e| e.to_string())
}
