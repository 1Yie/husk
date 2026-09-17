//! `App` — the single state struct iced's `update`/`view` operate on.
//!
//! Multi-session: `SessionManager` owns the live actors; `views` holds one
//! `SessionView` per session id so switching shows that session's stream
//! without a reload. `active_id` picks which view renders.

use std::collections::HashMap;

use crate::bridge::SessionManager;

/// One sidebar session entry.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub title: String,
    pub preview: String,
    pub active: bool,
    pub running: bool,
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
    /// Raw text (markdown source for agent messages).
    pub text: String,
    /// Parsed markdown items — kept in sync with `text` so `view` can render
    /// without re-borrowing. Rebuilt whenever `text` changes.
    pub items: Vec<iced::widget::markdown::Item>,
    pub reasoning: String,
    pub reasoning_open: bool,
    pub streaming: bool,
    /// Reasoning phase ended (the agent's answer started / turn finished) —
    /// flips the "Thinking…" header to a collapsed "Thought" label.
    pub thinking_done: bool,
    /// Selectable-text mode — the markdown body swaps to a read-only
    /// `text_editor::Content` so the user can drag-select + Ctrl+C. iced's
    /// markdown widget isn't selectable; this is the escape hatch.
    pub selectable: bool,
    /// The read-only editor content for selectable mode — kept in sync with
    /// `text` (rebuilt on toggle + on append while selectable).
    pub editor: iced::widget::text_editor::Content,
}

impl MessageRow {
    /// Construct a row, parsing `text` into markdown items.
    pub fn new(id: usize, role: Role, text: String) -> Self {
        let items = iced::widget::markdown::parse(&text).collect();
        let editor = iced::widget::text_editor::Content::with_text(&text);
        Self {
            id, role, text, items,
            reasoning: String::new(),
            reasoning_open: false,
            streaming: false,
            thinking_done: false,
            selectable: false,
            editor,
        }
    }

    /// Re-parse `text` → `items` after a streaming append; if selectable
    /// mode is on, refresh the editor content too so it tracks the stream.
    pub fn reparse(&mut self) {
        self.items = iced::widget::markdown::parse(&self.text).collect();
        if self.selectable {
            self.editor = iced::widget::text_editor::Content::with_text(&self.text);
        }
    }
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
    /// One-line target summary on the capsule (path/command/pattern).
    pub detail: String,
    /// Full tool output — only rendered when the capsule is expanded.
    pub output: String,
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

/// One entry in the rendered stream — an interleaved sequence of chat
/// messages and tool-call capsules, so the tool chain shows WHERE each call
/// happened in the turn (Codex-style) instead of a detached bottom strip.
#[derive(Debug)]
pub enum StreamItem {
    Message(MessageRow),
    Tool(StepRow),
}

/// One session's rendered state — the stream, steps, and pending approval
/// the UI shows when this session is active.
#[derive(Debug, Default)]
pub struct SessionView {
    /// Ordered stream: user/agent/system messages + tool capsules interleaved.
    pub stream: Vec<StreamItem>,
    /// Pending write/diff approval — `Some` ⇒ the matching capsule shows
    /// accept/deny inline. (Index refers into `stream`)
    pub pending: Option<ApprovalRow>,
    pub pending_diff: Vec<DiffLine>,
    /// Hunk-tracker feed — sidebar/slide-over surface (contract §workspace).
    #[allow(dead_code)]
    pub changed_files: Vec<String>,
    pub is_active: bool,
}

/// The root application state.
pub struct App {
    /// Multi-session kernel — actors, persistence, the tagged event queue.
    pub mgr: Option<SessionManager>,
    /// Per-session rendered state — switching just points `active_id` here.
    pub views: HashMap<i64, SessionView>,
    /// The session currently shown in the stream.
    pub active_id: i64,
    /// Sidebar rows (refreshed from `mgr.sidebar_rows()` on tick).
    pub sessions: Vec<SessionRow>,
    /// Stats bar — global to the window (active provider + active session's tokens).
    pub stats: StatsRow,
    /// Sandbox backend warning — global.
    pub sandbox_unsafe: bool,
    /// Composer text.
    pub input: String,
    /// Tick counter driving the loading-dots / caret animation.
    pub tick: u64,
    /// Scroll anchor for the stream's pin-to-bottom.
    pub stream_scroll: iced::widget::Id,
    /// Whether the user has scrolled away from the bottom (stop auto-pin).
    pub user_scrolled: bool,
}

impl App {
    /// Boot the live multi-session kernel.
    pub fn boot(mgr: SessionManager, sandbox_unsafe: bool) -> Self {
        let active_id = mgr.active_id;
        let sessions = mgr
            .sidebar_rows()
            .into_iter()
            .map(|(id, title, preview, active, running)| SessionRow {
                id,
                title,
                preview,
                active,
                running,
                timestamp: String::new(),
            })
            .collect();
        Self {
            mgr: Some(mgr),
            views: HashMap::new(),
            active_id,
            sessions,
            stats: StatsRow {
                agent_state: "Idle".into(),
                permission_mode: "default".into(),
                ..Default::default()
            },
            sandbox_unsafe,
            input: String::new(),
            tick: 0,
            stream_scroll: iced::widget::Id::unique(),
            user_scrolled: false,
        }
    }

    /// The active session's rendered view — `None` before any event lands
    /// (markdown items are `!Sync`, so no shared empty static is possible).
    pub fn view_for(&self, id: i64) -> Option<&SessionView> {
        self.views.get(&id)
    }

    /// Active session's view (mutable — create on demand).
    pub fn view_mut(&mut self) -> &mut SessionView {
        self.views.entry(self.active_id).or_default()
    }

    /// The currently-visible session's view (read) — `None` = empty stream.
    pub fn active_view(&self) -> Option<&SessionView> {
        self.view_for(self.active_id)
    }

    /// Static demo state for visual verification before the kernel attaches.
    pub fn demo() -> Self {
        let mut views = HashMap::new();
        views.insert(0, SessionView {
            stream: vec![
                StreamItem::Message(MessageRow::new(0, Role::User, "fix the borrow error in `engine.rs` — **urgent**".into())),
                StreamItem::Message({
                    let mut m = MessageRow::new(1, Role::Agent, "Reading `engine.rs` to locate the borrow conflict.\n\n- check `run_turn` borrow of `self.state`\n- try a `Mutex`".into());
                    m.reasoning = "scanning src/engine.rs…".into();
                    m.thinking_done = true;
                    m
                }),
                StreamItem::Tool(StepRow { id: 0, name: "smart_read".into(), state: StepState::Success, detail: "src/engine.rs · 48 lines".into(), expanded: false, output: String::new() }),
                StreamItem::Tool(StepRow { id: 1, name: "fuzzy_patch".into(), state: StepState::AwaitingConfirm, detail: "src/engine.rs · +14 −3".into(), expanded: false, output: String::new() }),
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
            is_active: false,
        });
        Self {
            mgr: None,
            views,
            active_id: 0,
            sessions: vec![
                SessionRow { id: 0, title: "fix borrow error".into(), preview: "Reading engine.rs…".into(), active: true, running: false, timestamp: "2m".into() },
                SessionRow { id: 1, title: "add streaming test".into(), preview: "fuzzy_patch applied".into(), active: false, running: false, timestamp: "1h".into() },
            ],
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
            input: String::new(),
            tick: 0,
            stream_scroll: iced::widget::Id::unique(),
            user_scrolled: false,
        }
    }
}

/// Rebuild a `SessionView`'s message list from a persisted `ChatMessage`
/// history — used when switching to a session whose in-memory view was
/// dropped. Tool/system rows map to stream entries; reasoning isn't
/// persisted so it collapses empty.
pub fn view_from_history(history: &[agent_llm::types::ChatMessage]) -> SessionView {
    let mut v = SessionView::default();
    for (i, m) in history.iter().enumerate() {
        // Tool result messages → a finished tool capsule in the chain.
        if m.role == agent_llm::types::Role::Tool {
            v.stream.push(StreamItem::Tool(StepRow {
                id: i,
                name: m.tool_call_id.clone().unwrap_or_else(|| "tool".into()),
                state: StepState::Success,
                detail: m.content.clone().unwrap_or_default().chars().take(60).collect(),
                expanded: false,
                output: String::new(),
            }));
            continue;
        }
        let role = match m.role {
            agent_llm::types::Role::User => Role::User,
            agent_llm::types::Role::Assistant => Role::Agent,
            _ => Role::System,
        };
        let text = m.content.clone().unwrap_or_default();
        if text.trim().is_empty() && m.tool_calls.is_none() {
            continue;
        }
        // An assistant message carrying tool_calls contributes a capsule per
        // call so the chain shows what was invoked, in order.
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                v.stream.push(StreamItem::Tool(StepRow {
                    id: i,
                    name: c.name.clone(),
                    state: StepState::Success,
                    detail: String::new(),
                    expanded: false,
                output: String::new(),
                }));
            }
        }
        if text.trim().is_empty() {
            continue;
        }
        let mut row = MessageRow::new(i, role, text);
        row.thinking_done = true;
        v.stream.push(StreamItem::Message(row));
    }
    v
}
