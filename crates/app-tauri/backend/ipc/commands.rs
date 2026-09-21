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
        let (cancel, steer_tx, decision, ask, permissions, agent_mode, cmd_tx) = {
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
        ask.answer(*request_id, answer.clone());
        return Ok(());
    }
    if let UiCommand::SetModel { provider, model } = &cmd {
        mgr.set_model(provider.clone(), model.clone());
    }
    if let UiCommand::SetThinkingLevel { level } = &cmd {
        mgr.set_thinking_level(level.clone());
    }
    if let UiCommand::SetPermissionMode { mode } = &cmd {
        mgr.set_permission_mode(mode.clone());
        if let Ok(mut gate) = permissions.write() {
            *gate = agent_kernel::permissions::PermissionGate::from_mode_str(mode);
        }
    }
    if let UiCommand::SetAgentMode { mode } = &cmd {
        mgr.set_agent_mode(mode.clone());
        if let Ok(mut slot) = agent_mode.write() {
            *slot = agent_kernel::mode::AgentMode::from_str(mode);
        }
    }
    cmd_tx.try_send(cmd).map_err(|e| e.to_string())
}
