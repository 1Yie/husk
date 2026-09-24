//! The two event enums that cross the kernel↔frontend channel.
//!
//! `AgentEvent` (sampler→session feedback) is kernel-internal, carries
//! `agent_llm::StreamChunk` and never crosses a transport — see
//! `agent-kernel/src/channels.rs`.

use serde::{Deserialize, Serialize};

/// Kernel state machine — mirrored to the UI so the status strip can render
/// the current phase without guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    ScanningWorkspace,
    /// Waiting on the first SSE byte.
    Reasoning,
    /// Text deltas flowing.
    StreamingToken,
    /// A write/destructive tool is paused for user review.
    AwaitingToolConfirmation {
        tool_name: String,
        diff_summary: String,
    },
    /// A plugin wants a capability beyond its manifest.
    AwaitingPluginConsent {
        plugin_id: String,
        capability: String,
    },
    /// P2: generalizes tool/plugin/capture approvals.
    AwaitingConsent {
        kind: String,
    },
    /// P2: CoW fork-and-verify planning.
    Branching {
        candidates: u8,
    },
    ExecutingTool {
        tool_name: String,
    },
    /// A compaction pass is in flight (auto near the window edge, or a
    /// manual `/compact`) — the UI shows a running card + status row.
    Compacting,
    Finished,
    Failed(String),
}

impl AgentState {
    /// Is the kernel mid-turn (any non-terminal, non-idle state)?
    pub fn is_active(&self) -> bool {
        !matches!(
            self,
            AgentState::Idle | AgentState::Finished | AgentState::Failed(_)
        )
    }

    /// Short human label for the status bar — deliberately NOT the Debug
    /// dump: `AwaitingToolConfirmation` carries a whole `diff_summary`
    /// (hundreds of chars) that would flood the bar, so map each variant
    /// to a compact, single-line description instead.
    pub fn label(&self) -> String {
        match self {
            AgentState::Idle => "idle".into(),
            AgentState::ScanningWorkspace => "scanning".into(),
            AgentState::Reasoning => "reasoning".into(),
            AgentState::StreamingToken => "streaming".into(),
            AgentState::AwaitingToolConfirmation { tool_name, .. } => {
                format!("confirm {tool_name}")
            }
            AgentState::AwaitingPluginConsent {
                plugin_id,
                capability,
            } => {
                format!("consent {plugin_id}:{capability}")
            }
            AgentState::AwaitingConsent { kind } => format!("consent {kind}"),
            AgentState::Branching { candidates } => format!("branching ×{candidates}"),
            AgentState::ExecutingTool { tool_name } => format!("running {tool_name}"),
            AgentState::Compacting => "compacting".into(),
            AgentState::Finished => "done".into(),
            AgentState::Failed(e) => format!("failed: {}", e.chars().take(30).collect::<String>()),
        }
    }
}

/// UI → kernel commands. `Steer` is the mid-turn injection — the engine
/// hot-patches the plan without resetting the state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiCommand {
    /// A fresh user prompt.
    Prompt { text: String },
    /// Mid-turn steering text (delivered as a user turn marked "The user
    /// interrupted:").
    Steer { text: String },
    /// Approval-card response for a pending tool call. `request_id`
    /// correlates the verdict to a specific `ApprovalRequested` — without it
    /// a stale click can be consumed by a *later* pending approval and
    /// approve a tool the user never reviewed (P1-a).
    ToolDecision { request_id: u64, approved: bool },
    /// Answer to a pending `QuestionAsked` card — `request_id` correlates
    /// the answer to the specific question, same staleness rule as
    /// `ToolDecision`.
    AnswerQuestion { request_id: u64, answer: String },
    /// Cancel the in-flight turn.
    Cancel,
    /// Hot-swap the active provider/model for the *next* turn.
    SetModel { provider: String, model: String },
    /// Kernel-internal: re-read the config and rebuild the running session's
    /// provider and model parameters in place. Sent by `SessionManager` after a
    /// config write — the UI never sends it.
    ReloadModel { provider: String, model: String },
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
    /// Undo/rewind the last turn's hunk set.
    UndoLastTurn,
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
    /// Park one follow-up prompt — appends to the queue, then drains right
    /// away when the session is idle. Distinct from `SetQueued` because a
    /// queue write that lands mid-turn is only processed after the turn:
    /// without this arm, a just-queued item would sit parked until some
    /// *later* turn ended. (`SetQueued` — the edit/remove/reorder path —
    /// deliberately never drains on its own.)
    Enqueue { text: String },
}

/// One offered answer on an `ask_question` card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Kernel → UI events. Every variant is displayable data — the frontend never
/// re-parses text to figure out what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiEvent {
    /// A state-machine transition.
    StateChanged(AgentState),
    /// The user prompt echoed back — the stream renders it as a user block.
    UserPrompt(String),
    /// A token delta (already throttled by the bridge — NOT per SSE chunk).
    /// `parent` is set on a delegated child's deltas so the UI nests them under
    /// that child's capsule instead of mixing them into the turn's draft.
    TextDelta {
        text: String,
        #[serde(default)]
        parent: Option<String>,
    },
    /// Reasoning-trace delta (separate visual stream; same `parent` rule).
    ReasoningDelta {
        text: String,
        #[serde(default)]
        parent: Option<String>,
    },
    /// A tool began executing. `args_preview` is a single-line summary of
    /// the call's target (a path / command / pattern) shown on the capsule —
    /// the full output only appears when the capsule is expanded.
    ToolCallStarted {
        name: String,
        args_preview: String,
        /// Set on calls emitted INSIDE another tool (`batch_execute`
        /// items) — the UI nests them under the parent capsule instead
        /// of counting them as top-level calls.
        #[serde(default)]
        parent: Option<String>,
    },
    /// A tool finished (or failed); `content` is the truncated model-facing
    /// text, `ui_type` is the optional typed card hint (`diff`/`table`/…).
    ToolCallFinished {
        name: String,
        ok: bool,
        content: String,
        ui_type: Option<String>,
        /// Same nesting marker as `ToolCallStarted::parent`.
        #[serde(default)]
        parent: Option<String>,
    },
    /// `ask_question` paused the turn for structured input — the card
    /// offers the model's options plus a free-text field. Resolved by
    /// `AnswerQuestion`.
    QuestionAsked {
        request_id: u64,
        question: String,
        options: Vec<AskOption>,
    },
    /// The matching `AnswerQuestion` resolved the parked card — emitted so
    /// the frontend can drop `pendingQuestion` from the view buffer itself,
    /// rather than hiding it behind a per-mount flag that a session switch
    /// would reset (the "answered card comes back" bug).
    QuestionAnswered { request_id: u64 },
    /// A destructive op paused for review — `diff` is the unified diff for
    /// the approval card; `fuzzy` flags an approximate patch match.
    /// `request_id` is the correlation token the frontend echoes back in
    /// `ToolDecision` so a stale verdict can't land on a different pending
    /// approval (P1-a).
    ApprovalRequested {
        request_id: u64,
        tool_name: String,
        diff: String,
        fuzzy: bool,
    },
    /// Final assistant text for the turn (complete, post-streaming).
    AssistantMessage(String),
    /// System/degrade notice (sampler retry, fallback, compaction…).
    SystemMessage(String),
    /// Token-usage update for the meter. `context_window` rides along so
    /// the UI can render fill percentage without re-deriving the bound.
    Usage {
        prompt_tokens: u32,
        completion_tokens: u32,
        context_window: u32,
        cached_tokens: u32,
    },
    /// Session-fatal error.
    Error(String),
    /// A compaction pass finished — older turns were folded into a summary
    /// note. The UI draws a compaction card (before → after tokens, how
    /// many messages were merged, the summary itself) and re-bases the
    /// context meter on `after_tokens` immediately, instead of waiting for
    /// the next sample's `Usage`.
    Compacted {
        before_tokens: u32,
        after_tokens: u32,
        removed_messages: u32,
        context_window: u32,
        /// `true` = the user ran `/compact`, `false` = the automatic trigger.
        /// The UI keeps an automatic pass inside the turn it happened in, but
        /// gives a manual one its own block — it is a user action between
        /// turns, not part of the previous answer.
        manual: bool,
        note: String,
    },
    /// A compaction pass started — emitted right before the summarization
    /// sample so the UI can draw the running card while the (potentially
    /// slow) pass runs. `manual` mirrors `Compacted`; no accounting yet.
    CompactionStarted { manual: bool },
    /// A `Retry` command rewound the session to the last user prompt —
    /// the UI drops the retried turn's items before the fresh
    /// `UserPrompt` echo lands on the same stream.
    TurnRetry,
    /// The session's queued-prompt list changed — emitted on every
    /// `SetQueued` write and after each item pops off the drain, so the
    /// composer always renders the kernel's list (single source of truth).
    /// Carries the full list — queue ops are rare and the list is small.
    QueuedPrompts { items: Vec<String> },
}
