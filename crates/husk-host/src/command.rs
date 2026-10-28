//! Husk's UI → session command set. The kernel's command channel is generic;
//! this enum is what Husk's frontends send and its session actor consumes.

use serde::{Deserialize, Serialize};

/// UI → kernel commands. `Steer` is the mid-turn injection — the engine
/// hot-patches the plan without resetting the state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiCommand {
    /// A fresh user prompt.
    Prompt { text: String },
    /// Mid-turn steering text, injected into the running turn as a tagged
    /// steer row (`ChatMessage::user_steer`), never as a new turn.
    Steer { text: String },
    /// Mid-run steering for one delegated subagent, addressed by the
    /// `child_id` its card carries. Only that child's owning session accepts it;
    /// anything else gets the same refusal as an unknown id.
    SteerChild { child_id: u64, text: String },
    /// Approval-card response for a pending tool call. `request_id`
    /// correlates the verdict to a specific `ApprovalRequested` — without it
    /// a stale click can be consumed by a *later* pending approval and
    /// approve a tool the user never reviewed (P1-a).
    ToolDecision {
        request_id: u64,
        approved: bool,
        /// Which approval-card action was picked (`link:session`,
        /// `link:workspace`). `None` is a plain 允许.
        #[serde(default)]
        action: Option<String>,
    },
    /// Whitelist a tool for the rest of this session — the gate's
    /// `rules.allow` gains `tool_name`, so subsequent calls of the same
    /// tool auto-approve instead of pausing again. Used by the
    /// computer-use approval dialog ("本会话允许"): granting once and then
    /// re-prompting on every `computer` action would defeat the flow.
    /// Never persisted and never touches `~/.config/husk`: the whitelist dies
    /// with the actor, and the kernel ends a `computer` grant at the end of
    /// each turn (`launch` is never covered by it).
    ApproveSessionTool { tool_name: String },
    /// Drop `tool_name` from the session whitelist — the inverse of
    /// `ApproveSessionTool`. The computer-use overlay's "停止" button
    /// sends this so the next `computer` call falls back to Ask (which
    /// re-pops the approval dialog instead of running silently).
    RevokeSessionTool { tool_name: String },
    /// Answer to a pending `QuestionAsked` card — `request_id` correlates
    /// the answer to the specific question, same staleness rule as
    /// `ToolDecision`.
    AnswerQuestion { request_id: u64, answer: String },
    /// Cancel the in-flight turn.
    Cancel,
    /// Hot-swap the active provider/model for the *next* turn.
    SetModel { provider: String, model: String },
    /// Kernel-internal: `config.toml` changed on disk — each live session
    /// re-reads it and rebuilds ITS OWN provider/model parameters in place.
    /// Sent by `SessionManager` after a config write — the UI never sends it.
    /// No payload: broadcasting the manager's mirrored pair used to re-point
    /// every actor at the ACTIVE session's model, and the next `persist_*`
    /// stamped that hijack into their metas.
    ReloadModel,
    /// Wake a durably accepted host preference; never accepted by `agent_cmd`.
    /// The actor reads the latest shared value, not stale queued toggle payloads.
    #[serde(skip)]
    SetFastMode { enabled: bool },
    /// Switch the permission mode for the *next* tool dispatch —
    /// `default` / `acceptEdits` / `auto` / `dontAsk` / `bypassPermissions`.
    /// The session rebuilds its `PermissionGate` from the label (same
    /// parsing as `PermissionGate::from_mode_str`) so the composer dropdown
    /// stays a thin label→kernel mapping with no policy logic UI-side.
    SetPermissionMode { mode: String },
    /// Switch the agent mode (`build` | `plan` | `goal`) — swaps the
    /// session's tool registry immediately (plan drops write tools, goal
    /// adds the completion contract) and rewrites the mode block inside
    /// the rendered system prompt.
    SetAgentMode { mode: String },
    /// Set the thinking/reasoning effort level (e.g. "off", "low", "medium", "high", "max").
    SetThinkingLevel { level: String },
    /// Set the global subagent default model — `delegate` children without a
    /// manifest `model:` run on it instead of the session's model. Empty
    /// `provider`/`model` clears it. Broadcast by `SessionManager` when the
    /// settings default changes; never persisted per session.
    SetSubagentModel { provider: String, model: String },
    /// Global thinking level for children spawned from this session. Empty
    /// `level` clears it (children follow the session's own level). Broadcast
    /// with `SetSubagentModel`; never persisted per session.
    SetSubagentThinking { level: String },
    /// Turn the `.blueprint/` architecture layer on or off for this session.
    SetBlueprintEnabled { enabled: bool },
    /// Undo/rewind the last turn's hunk set.
    UndoLastTurn,
    /// Hot-apply the memory preferences (`set_default_prefs` broadcasts
    /// this so a running session doesn't wait for its next spawn).
    /// `enabled=false` drops the store + distiller; `enabled=true` opens
    /// the store if absent and (re)builds the distiller per `distill`.
    SetMemory { enabled: bool, distill: bool },
    /// Regenerate the last turn — the session rewinds `history` to the
    /// last user prompt's position and re-runs it through the normal
    /// Prompt path. Idle-only; ignored while a turn is active.
    Retry,
    /// Replace the session's queued-prompt list — the composer's staging
    /// area for follow-ups parked until the current turn ends. Every list
    /// op (enqueue, edit, remove, reorder) collapses into this one write;
    /// the actor echoes the new list back via `QueuedPrompts` and mirrors
    /// it into `SessionMeta` so a reopened session restores it.
    SetQueued { items: Vec<String> },
    /// Apply a new auto-compaction ratio (fraction of the context window,
    /// 70/80/90% from the settings page) to a LIVE session. Broadcast by
    /// `SessionManager` when the workspace default changes — without it the
    /// new ratio only reached sessions spawned afterwards, so a running
    /// session kept the threshold it was born with.
    SetCompactAt { fraction: f32 },
    /// Apply new waiting bounds to a LIVE session: how long an approval card
    /// (`approval_secs`) or an `ask_question` card (`question_secs`) may sit
    /// unanswered before the kernel gives up, `0` = never. Broadcast by
    /// `SessionManager` when the settings default changes, for the same reason
    /// `SetCompactAt` exists.
    SetTimeouts { approval_secs: u64, question_secs: u64 },
    /// Park one follow-up prompt — appends to the queue, then drains right
    /// away when the session is idle. Distinct from `SetQueued` because a
    /// queue write that lands mid-turn is only processed after the turn:
    /// without this arm, a just-queued item would sit parked until some
    /// *later* turn ended. (`SetQueued` — the edit/remove/reorder path —
    /// deliberately never drains on its own.)
    Enqueue { text: String },
    /// Run the queue drain — nothing more. `Enqueue`/`SetQueued` mutate the
    /// session's shared deque directly through the IPC fast path (the command
    /// pump blocks behind a running turn, so a pump-carried write would hide
    /// the queue panel until turn end); this marker just wakes `drain_queue`
    /// once the pump is free. A no-op while a turn is live.
    DrainQueue,
}
