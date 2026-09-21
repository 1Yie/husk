//! The two event enums that cross the kernel↔frontend channel.
//!
//! Serialization is for the *wire* (app-cli/headless, future mesh); in-process
//! iced usage moves the same enums over `tokio::mpsc` — one contract, both
//! transports.
//!
//! `AgentEvent` (sampler→session internal feedback) is deliberately NOT here:
//! it's kernel-internal, carries `agent_llm::StreamChunk`, and never crosses
//! a transport — see `agent-kernel/src/channels.rs`.

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
    AwaitingToolConfirmation { tool_name: String, diff_summary: String },
    /// A plugin wants a capability beyond its manifest (Stage 9).
    AwaitingPluginConsent { plugin_id: String, capability: String },
    /// P2: generalizes tool/plugin/capture approvals.
    AwaitingConsent { kind: String },
    /// P2: CoW fork-and-verify planning.
    Branching { candidates: u8 },
    ExecutingTool { tool_name: String },
    /// Auto-compact in flight.
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
            AgentState::AwaitingPluginConsent { plugin_id, capability } => {
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
    /// Undo/rewind the last-turn hunk set (Stage 6 wires HunkTracker).
    UndoLastTurn,
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
    TextDelta(String),
    /// Reasoning-trace delta (separate visual stream).
    ReasoningDelta(String),
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
    QuestionAsked { request_id: u64, question: String, options: Vec<AskOption> },
    /// A destructive op paused for review — `diff` is the unified diff for
    /// the approval card; `fuzzy` flags an approximate patch match.
    /// `request_id` is the correlation token the frontend echoes back in
    /// `ToolDecision` so a stale verdict can't land on a different pending
    /// approval (P1-a).
    ApprovalRequested { request_id: u64, tool_name: String, diff: String, fuzzy: bool },
    /// Final assistant text for the turn (complete, post-streaming).
    AssistantMessage(String),
    /// System/degrade notice (sampler retry, fallback, compaction…).
    SystemMessage(String),
    /// Token-usage update for the meter. `context_window` rides along so
    /// the UI can render fill percentage without re-deriving the bound.
    Usage { prompt_tokens: u32, completion_tokens: u32, context_window: u32, cached_tokens: u32 },
    /// Session-fatal error.
    Error(String),
}
