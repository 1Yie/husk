//! `tokio::mpsc` channel setup — one channel per direction between actors.
//!
//! Actor topology (kernel-architecture.md):
//!
//! ```text
//! UI ──UiCommand──▶ SessionActor ──sampling──▶ SamplerActor (agent-llm)
//! UI ◀─UiEvent───── SessionActor ◀─chunks────
//! ```
//!
//! No shared `Mutex` between actors; `StreamThrottler`'s buffer (app-desktop)
//! is the only sanctioned shared cell.

use tokio::sync::mpsc;

use agent_ipc::{UiCommand, UiEvent};
use agent_llm::StreamChunk;

/// Internal sampler→session feedback — the kernel-internal counterpart that
/// carries `StreamChunk` plus sampler policy notices. NOT on the wire.
#[derive(Debug)]
pub enum AgentEvent {
    /// One normalized chunk from the provider.
    Chunk(StreamChunk),
    /// Sampler policy event (retry/degrade) → map to `UiEvent::SystemMessage`.
    SamplerNotice(String),
}

/// Bounded channel capacities — the engine applies backpressure rather than
/// letting a fast provider outrun a slow UI.
/// 8192: delta events are coalesced (~120 chars/event) but a fast provider
/// on a slow webview can still briefly outpace the forwarder — a deep
/// queue keeps control events (StateChanged/ApprovalRequested/Finished)
/// from being silently dropped by `try_send` behind a delta burst.
pub const UI_CHANNEL_CAP: usize = 8192;
pub const CMD_CHANNEL_CAP: usize = 32;
pub const AGENT_CHANNEL_CAP: usize = 1024;

/// UI-facing endpoints created by [`channels`].
pub struct UiChannels {
    /// UI → kernel commands.
    pub cmd_rx: mpsc::Receiver<UiCommand>,
    /// Kernel → UI events.
    pub event_tx: mpsc::Sender<UiEvent>,
    /// The frontend's send/recv halves (kept for the bridge layer).
    pub cmd_tx: mpsc::Sender<UiCommand>,
    pub event_rx: mpsc::Receiver<UiEvent>,
}

/// Kernel-internal sampling channel.
pub struct AgentChannels {
    pub event_tx: mpsc::Sender<AgentEvent>,
    pub event_rx: mpsc::Receiver<AgentEvent>,
}

/// Create the UI channel pair. The bridge (Stage 5) splits `UiChannels` into
/// its command-sender and event-receiver halves.
pub fn ui_channels() -> UiChannels {
    let (cmd_tx, cmd_rx) = mpsc::channel(CMD_CHANNEL_CAP);
    let (event_tx, event_rx) = mpsc::channel(UI_CHANNEL_CAP);
    UiChannels { cmd_rx, event_tx, cmd_tx, event_rx }
}

/// Create the sampler→session channel.
pub fn agent_channels() -> AgentChannels {
    let (event_tx, event_rx) = mpsc::channel(AGENT_CHANNEL_CAP);
    AgentChannels { event_tx, event_rx }
}
