//! `App` — the single state struct iced's `update`/`view` operate on.
//!
//! Multi-session: `SessionManager` owns the live actors; `views` holds one
//! `SessionView` per session id so switching shows that session's stream
//! without a reload. `active_id` picks which view renders.

use std::collections::HashMap;

use crate::bridge::SessionManager;
use crate::ui::animation::{self, Pulse};

/// One sidebar session entry.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: i64,
    pub title: String,
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
    /// Streaming-caret pulse — an `iced_anim` looping `Animated<f32>` that
    /// breathes the `▮` glyph's alpha while `streaming` is true. Advanced by
    /// the shared `Message::Tick` pump.
    pub caret: Pulse,
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
            caret: animation::pulse(500),
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
    /// The staged diff for a write/patch step — kept on the row so the
    /// expanded body still shows colored +/- lines after approval resolves
    /// (the view-level `pending_diff` clears on resolve).
    pub diff_lines: Vec<DiffLine>,
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
    /// Correlation id from `UiEvent::ApprovalRequested` — echoed back in
    /// `ToolDecision` so a stale click can't approve a different tool (P1-a).
    pub request_id: u64,
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
    /// Active provider name — reserved for a provider-switcher surface.
    #[allow(dead_code)]
    pub active_provider: String,
    pub active_model: String,
    /// Current git branch (session workspace) — status-bar right side.
    pub git_branch: String,
    /// True while running on a fallback_chain provider (status-bar ⚠).
    #[allow(dead_code)]
    pub degraded: bool,
}

/// One entry in the rendered stream — an interleaved sequence of chat
/// messages and tool-call capsules, so the tool chain shows WHERE each call
/// happened in the turn (Codex-style) instead of a detached bottom strip.
#[derive(Debug, Clone)]
pub enum StreamItem {
    Message(MessageRow),
    Tool(StepRow),
}

/// One session's rendered state — the stream, steps, and pending approval
/// the UI shows when this session is active. `Clone` so it can ride inside
/// `Message::SessionLoaded` (Message must be Clone for widget callbacks);
/// `Send` for the off-thread `view_from_history` rebuild, though it stays
/// `!Sync` via `markdown::Item`'s interior mutability.
#[derive(Debug, Clone)]
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
    /// Loading-dot pulses — three `iced_anim` looping `Animated<f32>`s,
    /// phase-offset so the dots wave. Advanced by `Message::Tick`.
    pub dot_pulses: Vec<Pulse>,
    /// SSE batching buffer — `TextDelta`/`ReasoningDelta` accumulate here
    /// and flush into the stream ONCE per `Tick` instead of re-parsing
    /// markdown per token. Decouples the SSE clock from the frame clock.
    pub pending_text: String,
    pub pending_reasoning: String,
}

impl Default for SessionView {
    fn default() -> Self {
        // Three dots, each a looping pulse phase-offset by a third of the
        // cycle so they read as a traveling wave rather than a sync blink.
        let dot_pulses = (0..3)
            .map(|i| {
                let mut p = animation::pulse(600);
                p.settle_at(i as f32 * 0.33);
                p
            })
            .collect();
        Self {
            stream: Vec::new(),
            pending: None,
            pending_diff: Vec::new(),
            changed_files: Vec::new(),
            is_active: false,
            dot_pulses,
            pending_text: String::new(),
            pending_reasoning: String::new(),
        }
    }
}

/// The root application state.
pub struct App {
    /// Multi-session kernel — actors, persistence, the tagged event queue.
    pub mgr: Option<SessionManager>,
    /// The `(session_id, UiEvent)` event tap — split out of `spawn()` so
    /// the Tick drain owns it (and `husk` can move its own copy into a
    /// forwarder thread). `Option` only for the `--mock` demo path.
    pub event_rx: Option<std::sync::mpsc::Receiver<(i64, agent_ipc::UiEvent)>>,
    /// Per-session rendered state — switching just points `active_id` here.
    pub views: HashMap<i64, SessionView>,
    /// The session currently shown in the stream.
    pub active_id: i64,
    /// Session whose history is being rebuilt off-thread — the stream shows
    /// a loading surface until `SessionLoaded` installs it.
    pub loading_session: Option<i64>,
    /// Tick counter — drives the braille spinner (`spinner_at`).
    pub tick_count: u64,
    /// Sidebar rows (refreshed from `mgr.sidebar_rows()` on tick).
    pub sessions: Vec<SessionRow>,
    /// Stats bar — global to the window (active provider + active session's tokens).
    pub stats: StatsRow,
    /// Sandbox backend warning — global.
    pub sandbox_unsafe: bool,
    /// Composer text.
    pub input: String,
    /// Current workspace root directory.
    pub workspace_root: std::path::PathBuf,
    /// Human-friendly workspace name (folder name).
    pub workspace_name: String,
    /// Recently opened workspaces.
    pub recent_workspaces: Vec<crate::bridge::RecentWorkspace>,
    /// The stream's scroll controller — follow/detach + snap decisions.
    pub scroll: super::chat::scroll::ScrollController,
    /// The OS window id — set on `WindowReady`, drives the custom titlebar's
    /// drag/min/max/close.
    pub window_id: Option<iced::window::Id>,
}

impl App {
    /// Boot the live multi-session kernel — takes the manager + its event
    /// receiver (the pair `SessionManager::spawn()` returns).
    pub fn boot(
        (mgr, event_rx): (SessionManager, std::sync::mpsc::Receiver<(i64, agent_ipc::UiEvent)>),
        sandbox_unsafe: bool,
    ) -> Self {
        let active_id = mgr.active_id;
        let sessions = mgr
            .sidebar_rows()
            .into_iter()
            .map(|(id, title, _preview, active, running)| SessionRow {
                id,
                title,
                active,
                running,
                timestamp: String::new(),
            })
            .collect();
        // Pull provider/model/branch before `mgr` moves into `self.mgr`.
        let (prov, model, branch) = (
            mgr.provider_name.clone(),
            mgr.model_name.clone(),
            git_branch(&mgr.workspace_root),
        );
        let workspace_root = mgr.workspace_root.clone();
        let workspace_name = workspace_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| workspace_root.to_string_lossy().to_string());
        let recent_workspaces = mgr.recent_workspaces();
        Self {
            mgr: Some(mgr),
            event_rx: Some(event_rx),
            views: HashMap::new(),
            active_id,
            loading_session: None,
            tick_count: 0,
            sessions,
            workspace_root,
            workspace_name,
            recent_workspaces,
            stats: StatsRow {
                agent_state: "Idle".into(),
                permission_mode: "default".into(),
                context_window: 256_000,
                active_provider: prov,
                active_model: model,
                git_branch: branch,
                ..Default::default()
            },
            sandbox_unsafe,
            input: String::new(),
            scroll: super::chat::scroll::ScrollController::default(),
            window_id: None,
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
                StreamItem::Tool(StepRow { id: 0, name: "smart_read".into(), state: StepState::Success, detail: "src/engine.rs · 48 lines".into(), expanded: false, output: String::new(), diff_lines: vec![] }),
                StreamItem::Tool(StepRow { id: 1, name: "fuzzy_patch".into(), state: StepState::AwaitingConfirm, detail: "src/engine.rs · +14 −3".into(), expanded: false, output: String::new(), diff_lines: vec![] }),
            ],
            pending: Some(ApprovalRow {
                step_id: 1,
                request_id: 1,
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
            ..Default::default()
        });
        Self {
            mgr: None,
            event_rx: None,
            views,
            active_id: 0,
            loading_session: None,
            tick_count: 0,
            sessions: vec![
                SessionRow { id: 0, title: "fix borrow error".into(), active: true, running: false, timestamp: "2m".into() },
                SessionRow { id: 1, title: "add streaming test".into(), active: false, running: false, timestamp: "1h".into() },
            ],
            stats: StatsRow {
                tokens_used: 12_400,
                context_window: 256_000,
                files_changed: 1,
                agent_state: "Idle".into(),
                permission_mode: "default".into(),
                active_provider: "grok".into(),
                active_model: "grok-4".into(),
                git_branch: "main".into(),
                degraded: false,
            },
            sandbox_unsafe: false,
            input: String::new(),
            workspace_root: std::path::PathBuf::from("/home/ichiyo/Workspace/agent-rs"),
            workspace_name: "agent-rs".into(),
            recent_workspaces: vec![
                crate::bridge::RecentWorkspace {
                    path: std::path::PathBuf::from("/home/ichiyo/Workspace/agent-rs"),
                    name: "agent-rs".into(),
                    last_opened: 0,
                },
            ],
            scroll: super::chat::scroll::ScrollController::default(),
            window_id: None,
        }
    }
}

/// Strip `[call: …]` text-protocol echoes from a persisted assistant
/// message — mirrors the kernel's `strip_call_echo` so a poisoned snapshot
/// doesn't render raw call syntax as chat text in `view_from_history`.
fn strip_call_echo_ui(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[call:") {
        let before = &rest[..start];
        out.push_str(before.strip_suffix('\n').unwrap_or(before));
        let after = &rest[start..];
        let end = after.find(']').map(|i| i + 1).unwrap_or(after.len());
        rest = &after[end..];
        if let Some(s) = rest.strip_prefix('\n') {
            rest = s;
        }
    }
    out.push_str(rest);
    out
}

/// Current git branch for the workspace — `git branch --show-current`,
/// empty when not a repo or git is unavailable.
pub(crate) fn git_branch(root: &std::path::Path) -> String {
    std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// Rebuild a `SessionView`'s message list from a persisted `ChatMessage`
/// history — used when switching to a session whose in-memory view was
/// dropped. Tool/system rows map to stream entries; reasoning isn't
/// persisted so it collapses empty.
pub fn view_from_history(history: &[agent_llm::types::ChatMessage]) -> SessionView {
    let mut v = SessionView::default();
    // call_id → tool name: a `Tool` result message only carries the call_id
    // (`call_abc…`), not the name — resolve it from the preceding assistant
    // message's `tool_calls` so the capsule shows the real name.
    let mut id_to_name: HashMap<String, String> = HashMap::new();
    for m in history {
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                id_to_name.insert(c.id.clone(), c.name.clone());
            }
        }
    }
    for (i, m) in history.iter().enumerate() {
        // System prompt — internal context, never rendered in the stream.
        if m.role == agent_llm::types::Role::System {
            continue;
        }
        // Tool result messages → a finished tool capsule in the chain.
        if m.role == agent_llm::types::Role::Tool {
            let call_id = m.tool_call_id.clone().unwrap_or_default();
            let name = id_to_name.get(&call_id).cloned().unwrap_or_else(|| "tool".into());
            let content = m.content.clone().unwrap_or_default();
            // Replay a unified diff the same way the live path does — a
            // persisted patch result still contains `@@`/`+`/`-` lines, so
            // parse them into `diff_lines` for the colored +/- view (the
            // capsule falls back to raw output when empty).
            let diff_lines = if content.contains("@@") || content.contains("+++") {
                let body_start = content
                    .lines()
                    .position(|l| l.starts_with("@@") || l.starts_with("--- ") || l.starts_with("diff "))
                    .unwrap_or(0);
                let body: String = content.lines().skip(body_start).collect::<Vec<_>>().join("\n");
                parse_unified_diff(&body)
            } else {
                Vec::new()
            };
            v.stream.push(StreamItem::Tool(StepRow {
                id: i,
                name,
                state: StepState::Success,
                detail: content.lines().next().unwrap_or("").chars().take(60).collect(),
                expanded: false,
                output: content,
                diff_lines,
            }));
            continue;
        }
        let role = match m.role {
            agent_llm::types::Role::User => Role::User,
            agent_llm::types::Role::Assistant => Role::Agent,
            _ => Role::System,
        };
        // Strip any persisted `[call:…]` text-protocol echo so a poisoned
        // snapshot doesn't render the raw call syntax as chat text.
        let text = if m.role == agent_llm::types::Role::Assistant {
            strip_call_echo_ui(&m.content.clone().unwrap_or_default())
        } else {
            m.content.clone().unwrap_or_default()
        };
        if text.trim().is_empty() && m.tool_calls.is_none() {
            continue;
        }
        // NOTE: assistant `tool_calls` do NOT get their own capsule here —
        // the matching `Role::Tool` result message (rendered above) already
        // produces the finished capsule with the real output. Emitting one
        // per tool_call too would double-render every call: an empty
        // name-only capsule (no output) followed by the real result capsule.
        // `tool_calls` are only read during the id→name pre-scan.
        if text.trim().is_empty() {
            continue;
        }
        let mut row = MessageRow::new(i, role, text);
        row.thinking_done = true;
        v.stream.push(StreamItem::Message(row));
    }
    v
}

/// Split a unified diff into `DiffLine` rows — shared by the live
/// `ToolCallFinished`/`ApprovalRequested` path (update.rs) and
/// `view_from_history`'s persisted-result replay so both render the same
/// colored +/- view.
pub fn parse_unified_diff(diff: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old_ln, mut new_ln) = (0i32, 0i32);
    for line in diff.lines() {
        if line.starts_with("@@") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                old_ln = parts[1].trim_start_matches('-').split(',').next()
                    .and_then(|s| s.parse().ok()).unwrap_or(0);
                new_ln = parts[2].trim_start_matches('+').split(',').next()
                    .and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            continue;
        }
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("diff ") {
            continue;
        }
        let (kind, o, n) = if line.starts_with('+') {
            let v = (DiffKind::Add, -1, new_ln);
            new_ln += 1;
            v
        } else if line.starts_with('-') {
            let v = (DiffKind::Delete, old_ln, -1);
            old_ln += 1;
            v
        } else {
            let v = (DiffKind::Context, old_ln, new_ln);
            old_ln += 1;
            new_ln += 1;
            v
        };
        out.push(DiffLine {
            kind,
            content: line.chars().skip(1).collect(),
            old_lineno: o,
            new_lineno: n,
        });
    }
    out
}
