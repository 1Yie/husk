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
    /// Approval-card response for a pending tool call.
    ToolDecision { approved: bool },
    /// Cancel the in-flight turn.
    Cancel,
    /// Hot-swap the active provider/model for the *next* turn.
    SetModel { provider: String, model: String },
    /// Undo/rewind the last-turn hunk set (Stage 6 wires HunkTracker).
    UndoLastTurn,
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
    /// A tool began executing.
    ToolCallStarted { name: String },
    /// A tool finished (or failed); `content` is the truncated model-facing
    /// text, `ui_type` is the optional typed card hint (`diff`/`table`/…).
    ToolCallFinished { name: String, ok: bool, content: String, ui_type: Option<String> },
    /// A destructive op paused for review — `diff` is the unified diff for
    /// the approval card; `fuzzy` flags an approximate patch match.
    ApprovalRequested { tool_name: String, diff: String, fuzzy: bool },
    /// Final assistant text for the turn (complete, post-streaming).
    AssistantMessage(String),
    /// System/degrade notice (sampler retry, fallback, compaction…).
    SystemMessage(String),
    /// Token-usage update for the meter.
    Usage { prompt_tokens: u32, completion_tokens: u32 },
    /// Session-fatal error.
    Error(String),
}
