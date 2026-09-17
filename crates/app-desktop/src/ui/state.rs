//! `App` — the single state struct iced's `update`/`view` operate on.
//! Flat `Vec`s of row data; kernel `UiEvent`s mutate them in `update()`.

use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use agent_ipc::UiCommand;

/// One sidebar session entry.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub title: String,
    pub preview: String,
    pub active: bool,
    /// Relative time label — reserved for the session-browser detail pass.
    #[allow(dead_code)]
    pub timestamp: String,
}

/// One chat-stream message.
#[derive(Debug, Clone)]
pub struct MessageRow {
    /// Stable identity — reserved for future virtualized list keys.
    #[allow(dead_code)]
    pub id: usize,
    pub role: Role,
    pub text: String,
    pub reasoning: String,
    pub reasoning_open: bool,
    pub streaming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    System,
}

/// One tool-call capsule.
#[derive(Debug, Clone)]
pub struct StepRow {
    /// Stable identity — reserved for future virtualization.
    #[allow(dead_code)]
    pub id: usize,
    pub name: String,
    pub state: StepState,
    pub detail: String,
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Running,
    AwaitingConfirm,
    Success,
    Error,
    Denied,
}

/// A parsed unified-diff line for the inline/pending diff view.
/// A parsed unified-diff line — rendered by the pending-approval view.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields read when the inline diff body lands
pub struct DiffLine {
    pub kind: DiffKind,
    pub content: String,
    pub old_lineno: i32,
    pub new_lineno: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Add,
    Delete,
    Context,
}

/// The pending approval — `Some` ⇒ the matching capsule shows accept/deny.
/// `risk`/`title`/`diff_text`/`fuzzy`/`step_id` are read by the approval
/// banner + risk-tier styling when that surface lands (contract §pending).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ApprovalRow {
    pub step_id: usize,
    pub tool_name: String,
    pub title: String,
    pub diff_text: String,
    pub fuzzy: bool,
    pub risk: String,
}

/// Stats bar feed. `degraded` is surfaced when a provider falls back.
#[derive(Debug, Clone, Default)]
pub struct StatsRow {
    pub tokens_used: u32,
    pub context_window: u32,
    pub files_changed: u32,
    pub agent_state: String,
    pub permission_mode: String,
    pub active_provider: String,
    pub active_model: String,
    /// True while running on a fallback_chain provider (status-bar ⚠).
    #[allow(dead_code)]
    pub degraded: bool,
}

/// The root application state.
pub struct App {
    pub sessions: Vec<SessionRow>,
    pub messages: Vec<MessageRow>,
    pub active_steps: Vec<StepRow>,
    pub pending: Option<ApprovalRow>,
    pub pending_diff: Vec<DiffLine>,
    /// Hunk-tracker feed — rendered in the sidebar/slide-over (contract).
    #[allow(dead_code)]
    pub changed_files: Vec<String>,
    pub stats: StatsRow,
    pub sandbox_unsafe: bool,
    pub is_active: bool,
    pub input: String,
    /// Tick counter driving the loading-dots / caret animation.
    pub tick: u64,
    /// Kernel command channel — UI → kernel (prompts, cancel, …).
    pub cmd_tx: Option<tokio::sync::mpsc::Sender<UiCommand>>,
    /// Approval decision slot — kernel polls this; can't queue behind a turn.
    pub decision: Arc<Mutex<Option<bool>>>,
    /// Mid-turn steer channel.
    pub steer_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// Kernel-event forwarder — the subscription drains this receiver.
    pub event_rx: Option<Arc<Mutex<std_mpsc::Receiver<agent_ipc::UiEvent>>>>,
    /// Scroll anchor for the stream's pin-to-bottom.
    pub stream_scroll: iced::widget::Id,
    /// Whether the user has scrolled away from the bottom (stop auto-pin).
    pub user_scrolled: bool,
}

impl App {
    /// Boot a live kernel-backed app. `bridge::spawn_kernel` fills in
    /// `cmd_tx`/`decision`/`event_rx` before `iced` starts the loop.
    pub fn boot(
        cmd_tx: tokio::sync::mpsc::Sender<UiCommand>,
        decision: Arc<Mutex<Option<bool>>>,
        steer_tx: tokio::sync::mpsc::Sender<String>,
        event_rx: std_mpsc::Receiver<agent_ipc::UiEvent>,
        stats: StatsRow,
        sandbox_unsafe: bool,
    ) -> Self {
        Self {
            sessions: vec![SessionRow {
                id: 0,
                title: "current session".into(),
                preview: "live kernel".into(),
                active: true,
                timestamp: "now".into(),
            }],
            messages: Vec::new(),
            active_steps: Vec::new(),
            pending: None,
            pending_diff: Vec::new(),
            changed_files: Vec::new(),
            stats,
            sandbox_unsafe,
            is_active: false,
            input: String::new(),
            tick: 0,
            cmd_tx: Some(cmd_tx),
            decision,
            steer_tx: Some(steer_tx),
            event_rx: Some(Arc::new(Mutex::new(event_rx))),
            stream_scroll: iced::widget::Id::unique(),
            user_scrolled: false,
        }
    }

    /// Static demo state for visual verification before the kernel attaches.
    pub fn demo() -> Self {
        Self {
            sessions: vec![
                SessionRow { id: 0, title: "fix borrow error".into(), preview: "Reading engine.rs…".into(), active: true, timestamp: "2m".into() },
                SessionRow { id: 1, title: "add streaming test".into(), preview: "fuzzy_patch applied".into(), active: false, timestamp: "1h".into() },
            ],
            messages: vec![
                MessageRow { id: 0, role: Role::User, text: "fix the borrow error in engine.rs".into(), reasoning: String::new(), reasoning_open: false, streaming: false },
                MessageRow { id: 1, role: Role::Agent, text: "Reading the engine module to locate the borrow conflict.".into(), reasoning: "scanning src/engine.rs…".into(), reasoning_open: false, streaming: false },
            ],
            active_steps: vec![
                StepRow { id: 0, name: "smart_read".into(), state: StepState::Success, detail: "src/engine.rs · 48 lines".into(), expanded: false },
                StepRow { id: 1, name: "fuzzy_patch".into(), state: StepState::AwaitingConfirm, detail: "src/engine.rs · +14 −3".into(), expanded: false },
            ],
            pending: Some(ApprovalRow {
                step_id: 1,
                tool_name: "fuzzy_patch".into(),
                title: "Edit crates/agent-kernel/src/engine.rs".into(),
                diff_text: String::new(),
                fuzzy: false,
                risk: "normal".into(),
            }),
            pending_diff: vec![
                DiffLine { kind: DiffKind::Context, content: "    pub async fn run_turn(".into(), old_lineno: 40, new_lineno: 40 },
                DiffLine { kind: DiffKind::Delete, content: "        self.state = State::Idle;".into(), old_lineno: 41, new_lineno: -1 },
                DiffLine { kind: DiffKind::Add, content: "        self.state = State::Running;".into(), old_lineno: -1, new_lineno: 41 },
            ],
            changed_files: vec!["crates/agent-kernel/src/engine.rs".into()],
            stats: StatsRow {
                tokens_used: 12_400,
                context_window: 256_000,
                files_changed: 1,
                agent_state: "Idle".into(),
                permission_mode: "default".into(),
                active_provider: "grok".into(),
                active_model: "grok-4".into(),
                degraded: false,
            },
            sandbox_unsafe: false,
            is_active: false,
            input: String::new(),
            tick: 0,
            cmd_tx: None,
            decision: Arc::new(Mutex::new(None)),
            steer_tx: None,
            event_rx: None,
            stream_scroll: iced::widget::Id::unique(),
            user_scrolled: false,
        }
    }
}
