//! `commands.rs` — slash-command registry, phase 1 of the extension pipeline.
//!
//! Contract (plugin-system.md §Commands): input starting with `/` is
//! intercepted **before** the ReAct loop — zero tokens spent. Built-ins ship
//! first; MCP `prompts/*` and WASM `command_execute` register under
//! `plugin_id:name` when those runtimes land.
//!
//! Dispatch order: `CommandRegistry::try_run` returns `Some(result)` when a
//! `/x` matched a command — the session turns `ControlAction` into state
//! changes and `Reply`/`FeedToAgent` into messages without touching the LLM.

use agent_ipc::UiEvent;
use agent_llm::types::ChatMessage;

/// What a command produced — the session maps it to side effects.
#[derive(Debug)]
pub enum CommandResult {
    /// Local echo — append as a system message, no LLM call.
    Reply(String),
    /// Session control — reset/switch-model/undo…
    Control(ControlOp),
    /// Wrap the result into a normal prompt and continue the turn.
    FeedToAgent(String),
}

/// State-changing commands the session executes.
#[derive(Debug)]
pub enum ControlOp {
    /// `/clear` — wipe history back to the system prompt.
    ClearHistory,
    /// `/compact` — force a compaction pass.
    Compact,
    /// `/undo` — reverse the last turn's writes (HunkTracker).
    UndoLastTurn,
    /// `/model <provider>/<model>` — hot-swap for the next turn.
    SetModel { provider: String, model: String },
}

/// Context a command needs — session passes itself in pieces.
pub struct CommandCtx<'a> {
    pub history: &'a mut Vec<ChatMessage>,
    pub permission_mode: &'a str,
    pub workspace_root: &'a std::path::Path,
    /// Emit a UI event (status/system message) — a `Sender` keeps the ctx
    /// `Send` so `handle` futures can `tokio::spawn`.
    pub ui_tx: &'a tokio::sync::mpsc::Sender<UiEvent>,
}

/// The registry — built-ins first; plugin commands register under
/// `plugin_id:name`.
pub struct CommandRegistry;

impl CommandRegistry {
    /// Intercept a `/x` input. Returns `Some(result)` when it was a command,
    /// `None` when the text should fall through to the LLM (unknown `/x`
    /// gets a hint but still feeds through as a prompt — the contract).
    pub async fn try_run(
        text: &str,
        ctx: &mut CommandCtx<'_>,
    ) -> Option<CommandResult> {
        let t = text.trim();
        if !t.starts_with('/') {
            return None;
        }
        let (name, args) = t.split_once(' ').map(|(n, a)| (n, a.trim()))
            .unwrap_or((t, ""));
        let name = name.trim_start_matches('/');

        Some(match name {
            "clear" => CommandResult::Control(ControlOp::ClearHistory),
            "compact" => CommandResult::Control(ControlOp::Compact),
            "undo" => CommandResult::Control(ControlOp::UndoLastTurn),
            "model" => {
                // `/model provider/model` or bare `/model provider model`
                let parts: Vec<&str> = args.splitn(2, |c| c == '/' || c == ' ').collect();
                let (provider, model) = match parts.as_slice() {
                    [p, m] => ((*p).to_string(), (*m).to_string()),
                    [m] => ("".to_string(), (*m).to_string()),
                    _ => return Some(CommandResult::Reply("usage: /model <provider>/<model>".into())),
                };
                CommandResult::Control(ControlOp::SetModel { provider, model })
            }
            "diff" => {
                // `/diff` — report the turn's changed files via hunk tracker.
                // The session owns the tracker; command returns a reply
                // asking the session to emit the file list (ControlOp could
                // carry it, but Reply keeps the boundary simple).
                CommandResult::Reply(
                    "[/diff] file-change report is emitted by the session".into(),
                )
            }
            _ => {
                // Unknown /x → hint + fall through as a normal prompt.
                let _ = ctx.ui_tx.try_send(UiEvent::SystemMessage(format!(
                    "`/{name}` not a command — sent as prompt"
                )));
                return None;
            }
        })
    }
}
