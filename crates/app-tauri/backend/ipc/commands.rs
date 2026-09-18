//! `agent_cmd` — the single invoke for all `UiCommand`s.
//!
//! One command, the enum carries the intent: `Prompt` / `Steer` /
//! `ToolDecision` / `Cancel` / `SetModel` / `UndoLastTurn`. `Cancel` and
//! `Steer`/`ToolDecision` bypass the command pump (direct flag / channel)
//! so they aren't queued behind a running `run_turn`.

use agent_ipc::UiCommand;
use tauri::State;

use crate::kernel::KernelState;

#[tauri::command]
pub fn agent_cmd(state: State<'_, KernelState>, cmd: UiCommand) -> Result<(), String> {
    let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
    let Some(handle) = mgr.active() else {
        return Err("no active session".into());
    };
    // `Cancel` bypasses the command pump (direct flag) so it isn't queued
    // behind a running `run_turn`.
    if matches!(cmd, UiCommand::Cancel) {
        handle.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(());
    }
    if let UiCommand::Steer { text } = &cmd {
        handle.steer_tx.try_send(text.clone()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let UiCommand::ToolDecision { request_id, approved } = &cmd {
        *handle.decision.lock().map_err(|e| e.to_string())? = Some((*request_id, *approved));
        return Ok(());
    }
    let cmd_tx = handle.cmd_tx.clone();
    if let UiCommand::SetModel { provider, model } = &cmd {
        mgr.set_model(provider.clone(), model.clone());
    }
    if let UiCommand::SetThinkingLevel { level } = &cmd {
        mgr.set_thinking_level(level.clone());
    }
    cmd_tx.try_send(cmd).map_err(|e| e.to_string())
}
