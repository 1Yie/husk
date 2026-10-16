//! `agent_cmd` — the single invoke for all `UiCommand`s.
//!
//! One command, the enum carries the intent. `Cancel`, `Steer` and `ToolDecision`
//! bypass the command pump (direct flag / channel) so they are not queued behind a
//! running turn.

use agent_ipc::UiEvent;
use husk_host::command::UiCommand;
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
pub async fn get_ui_stats(
    state: State<'_, KernelState>,
) -> Result<agent_kernel::channels::UiStatsSnapshot, String> {
    // The StreamHealth chip polls this on an interval — a SYNC command waits
    // on the main thread while a worker holds the manager through an MCP
    // handshake, freezing the UI. Same `spawn_blocking` rule as
    // `agent_session`.
    let kernel = state.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mgr = kernel.lock().map_err(|e| e.to_string())?;
        Ok(mgr.ui_stats())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn agent_cmd(state: State<'_, KernelState>, cmd: UiCommand) -> Result<(), String> {
    // Sync commands run on the main thread — locking the manager here while a
    // spawn_blocking worker holds it through an MCP handshake freezes the UI.
    // Forward off-thread instead; the mutex keeps per-sender ordering.
    let kernel = state.inner().0.clone();
    tauri::async_runtime::spawn_blocking(move || agent_cmd_inner(kernel, cmd))
        .await
        .map_err(|e| e.to_string())?
}

fn agent_cmd_inner(
    kernel: std::sync::Arc<std::sync::Mutex<husk_host::session_manager::SessionManager>>,
    cmd: UiCommand,
) -> Result<(), String> {
    let mut mgr = kernel.lock().map_err(|e| e.to_string())?;
    let (cancel, steer_tx, decision, ask, permissions, agent_mode, model, ui, cmd_tx, queue) = {
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
            handle.model.clone(),
            handle.ui.clone(),
            handle.cmd_tx.clone(),
            handle.queue.clone(),
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
    if let UiCommand::SteerChild { child_id, text } = &cmd {
        // The active session owns the child; the client names only the child.
        mgr.steer_active_child(*child_id, text.clone())?;
        return Ok(());
    }

    if let UiCommand::ToolDecision {
        request_id,
        approved,
    } = &cmd
    {
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
            let _ = ui.send(UiEvent::QuestionAnswered {
                request_id: *request_id,
            });
        }
        return Ok(());
    }
    // Queue ops bypass the pump too: the deque is shared with the actor, so
    // an `Enqueue`/`SetQueued` applies instantly — a pump-carried write would
    // sit behind the running turn and the `QueuedPrompts` echo (the queue
    // panel's only data source) would not surface until the turn ended.
    // `Enqueue` still forwards a `DrainQueue` marker so the parked item runs
    // the moment a slot frees up; `SetQueued` is a pure list write.
    if let UiCommand::Enqueue { text } = &cmd {
        let session_id = mgr.active_id;
        let items = {
            let mut q = queue.lock().map_err(|e| e.to_string())?;
            q.push_back(text.clone());
            q.iter().cloned().collect::<Vec<_>>()
        };
        let _ = ui.send(UiEvent::QueuedPrompts {
            items: items.clone(),
        });
        mgr.set_session_queued(session_id, items);
        cmd_tx
            .try_send(UiCommand::DrainQueue)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let UiCommand::SetQueued { items } = &cmd {
        let session_id = mgr.active_id;
        *queue.lock().map_err(|e| e.to_string())? = items.clone().into();
        let _ = ui.send(UiEvent::QueuedPrompts {
            items: items.clone(),
        });
        mgr.set_session_queued(session_id, items.clone());
        return Ok(());
    }
    // Composer settings are PER-SESSION: the command goes only to the active
    // actor, which applies it to its own engine and persists it into its own
    // SessionMeta. They deliberately do NOT touch the manager-level workspace
    // default (`mgr.set_*`) — that's what used to leak one session's
    // model/mode/level into every other session's display & spawn.
    // `SetPermissionMode`/`SetAgentMode` still write the live handle slots here
    // so a mid-turn swap applies at the next dispatch rather than after the turn.
    if let UiCommand::SetPermissionMode { mode } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            // In place: rebuilding the gate would drop the session's allow rules.
            gate.set_mode(agent_kernel::permissions::PermissionMode::from_str(mode));
        }
    }
    if let UiCommand::SetAgentMode { mode } = &cmd {
        if let Ok(mut slot) = agent_mode.write() {
            *slot = husk_host::mode::AgentMode::from_str(mode)
                .as_str()
                .to_string();
        }
    }
    // Per-session tool whitelisting (computer-use's "本会话允许"): write
    // the live gate directly so the next dispatch sees it — forwarding
    // through `cmd_tx` would only apply it at the actor's next `handle`,
    // and a `computer` call paused in AwaitingToolConfirmation is waiting
    // on the *decision* slot, not the command pump, so the gate has to be
    // hot right now.
    if let UiCommand::ApproveSessionTool { tool_name } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            gate.allow_tool_for_session(tool_name);
        }
        // Notify the UI — the computer-use overlay mounts off this event.
        let _ = ui.send(UiEvent::SessionToolWhitelisted {
            tool_name: tool_name.clone(),
            stopped: false,
        });
        return Ok(());
    }
    // The overlay's "停止" button — same direct-write path. Session-level
    // revoke so the NEXT `computer` call drops back to Ask; any running
    // call isn't retro-cancelled.
    if let UiCommand::RevokeSessionTool { tool_name } = &cmd {
        if let Ok(mut gate) = permissions.write() {
            gate.revoke_tool_for_session(tool_name);
        }
        let _ = ui.send(UiEvent::SessionToolWhitelisted {
            tool_name: tool_name.clone(),
            stopped: true,
        });
        return Ok(());
    }
    // `SetModel` still travels the pump (the provider rebuild belongs to the
    // actor), but the live slot is written as soon as it is queued: behind a
    // running turn the pump only drains at turn end, and until then every
    // `model_info` read snapped the composer back to the old model. Written
    // after the send, so a full queue never advertises a switch that won't run.
    let pick = match &cmd {
        UiCommand::SetModel { provider, model } => Some((provider.clone(), model.clone())),
        _ => None,
    };
    cmd_tx.try_send(cmd).map_err(|e| e.to_string())?;
    if let Some(pick) = pick {
        if let Ok(mut slot) = model.write() {
            *slot = pick;
        }
    }
    Ok(())
}
