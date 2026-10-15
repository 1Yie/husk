//! App state + event handlers: committed cells flush to terminal scrollback,
//! in-flight stream items render live inside the inline viewport.

use std::io;
use std::sync::{Arc, Mutex};

use agent_ipc::AgentState;
use husk_host::command::UiCommand;
use agent_kernel::channels::UiReceiver;

use ratatui::text::Line;
use tokio::sync::mpsc;

use crate::composer::Composer;
use crate::modals::Modal;
use crate::{cells::Cell, transcript};

/// One entry in the live (pre-commit) transcript — appended in event order.
pub enum LiveItem {
    /// Accumulating assistant text run (rendered plain while streaming;
    /// the committed cell re-renders as markdown).
    Text(StreamText),
    /// Model reasoning, shown dim+italic.
    Reasoning(StreamText),
    /// Delegated/child-agent stream chunk (parent-sourced deltas).
    Delegate(StreamText),
    /// A top-level tool call, live until its `ToolCallFinished` commits it.
    /// Batch children are collected here and committed with the parent.
    Tool {
        name: String,
        preview: String,
        started: std::time::Instant,
        children: Vec<ChildTool>,
    },
}

/// One `batch_execute` child, kept until its parent commits.
#[derive(Debug)]
pub struct ChildTool {
    pub name: String,
    pub preview: String,
    /// `false` until the child's `ToolCallFinished` arrives.
    pub done: bool,
    pub ok: bool,
    pub ui_type: Option<String>,
    pub tail: Vec<String>,
}

/// Incrementally wrap deltas; only a width change rebuilds earlier rows.
pub struct StreamText {
    text: String,
    rows: Vec<String>,
    column: usize,
    width: usize,
    prefix: &'static str,
}

impl StreamText {
    pub fn new(text: &str, prefix: &'static str, width: u16) -> Self {
        let mut this = Self {
            text: String::new(),
            rows: vec![String::new()],
            column: 0,
            width: width.max(8) as usize,
            prefix,
        };
        this.append_wrapped(prefix);
        this.push_str(text);
        this
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn push_str(&mut self, text: &str) {
        self.text.push_str(text);
        self.append_wrapped(text);
    }

    fn append_wrapped(&mut self, text: &str) {
        use unicode_width::UnicodeWidthChar;
        for ch in text.chars() {
            if ch == '\n' {
                self.rows.push(String::new());
                self.column = 0;
                self.append_wrapped(self.prefix);
                continue;
            }
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if self.column + w > self.width {
                self.rows.push(String::new());
                self.column = 0;
            }
            self.rows.last_mut().unwrap().push(ch);
            self.column += w;
        }
    }

    fn rows(&mut self, width: u16) -> &[String] {
        let width = width.max(8) as usize;
        if self.width != width {
            self.width = width;
            self.rows.clear();
            self.rows.push(String::new());
            self.column = 0;
            self.append_wrapped(self.prefix);
            let text = std::mem::take(&mut self.text);
            self.append_wrapped(&text);
            self.text = text;
        }
        let end = self.rows.len() - usize::from(self.text.ends_with('\n') || self.text.is_empty());
        &self.rows[..end]
    }
}

/// One task-list row parsed from the `todo` tool's rendered output.
pub struct TodoItem {
    pub id: u32,
    pub text: String,
    pub done: bool,
}

/// Auto-approve approvals/questions without opening the modal (`--yes`).
pub struct App {
    pub cmd_tx: mpsc::Sender<UiCommand>,
    pub ev_rx: UiReceiver,

    /// Fast-path handles — the turn engine polls these directly, so
    /// decisions/answers/cancels must NOT go through `cmd_tx` (the pump is
    /// blocked inside `run_prompt` for the whole turn; the IPC bridge
    /// bypasses the channel the same way).
    pub(crate) decision_slot: Arc<Mutex<Option<(u64, bool)>>>,
    pub(crate) ask_channel: Arc<agent_kernel::tools::registry::AskChannel>,
    pub(crate) cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) queue: Arc<Mutex<std::collections::VecDeque<String>>>,

    pub state: AgentState,
    pub composer: Composer,
    pub modal: Option<Modal>,

    /// Ordered in-flight transcript — every stream/tool event appends in
    /// arrival order so the live region reads like the final transcript
    /// instead of pinning tool lines below the text (or vice versa).
    /// Text/Reasoning accumulate per contiguous run; Tool carries its own
    /// timer and resolves in place on finish.
    pub live: Vec<LiveItem>,
    /// In-flight assistant text (committed as markdown on AssistantMessage).
    pub active_text: String,
    /// Kernel emits `StateChanged(Finished)` BEFORE `AssistantMessage`
    /// (engine.rs: set_state → send). Flushing the stream buffer at the
    /// state event would double-commit, so terminal states only ARM a
    /// deadline — AssistantMessage wins the race, the tick is the fallback
    /// for cancel/fail paths where no message follows.
    pub(crate) pending_flush: Option<std::time::Instant>,
    /// Per-turn elapsed display — set when the state machine goes active.
    pub turn_started: Option<std::time::Instant>,

    pub queued: Vec<String>,
    pub usage: Option<(u32, u32, u32)>,

    pub provider_name: String,
    pub model: String,
    pub sandbox_id: String,
    pub auto_yes: bool,

    /// Display/status state the composer doesn't carry — updated by the
    /// slash commands that set them.
    pub workspace: std::path::PathBuf,
    pub permission_mode: String,
    pub agent_mode: String,
    pub thinking_level: Option<String>,
    /// provider → model ids, for `/model`'s two-stage picker.
    pub(crate) model_catalog: Vec<(String, Vec<String>)>,
    /// `/resume` picker resolved — main polls this and rebuilds the actor.
    pub(crate) resume_request: Option<i64>,
    /// Bottom status bar: (branch, dirty-count) refreshed every ~2s on tick.
    pub git_info: Option<(String, usize)>,
    pub(crate) git_at: std::time::Instant,
    /// Live session row id — `/resume` pre-selects it, `/new` allocates a
    /// fresh one, the exit hint prints `husk-tui -s <id>`.
    pub session_id: i64,
    /// `/new` requested — main allocates the id and swaps the actor.
    pub(crate) new_request: bool,
    /// `/` autocomplete: candidate cursor + Esc-dismissed flag (cleared on
    /// any text edit — see on_key).
    pub slash_sel: usize,
    pub slash_dismissed: bool,
    /// Persistent task list — parsed out of `todo` tool results and shown
    /// as a strip above the composer until cleared/replaced.
    pub todos: Vec<TodoItem>,

    pub spinner: usize,
    pub quit: bool,
    pub exit_code: i32,

    /// Current terminal width — updated on draw/resize so height math and
    /// scrollback wrap agree.
    pub(crate) width: u16,
    /// Lines rendered into scrollback next flush.
    pub(crate) pending: Vec<Line<'static>>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cmd_tx: mpsc::Sender<UiCommand>,
        ev_rx: UiReceiver,
        decision_slot: Arc<Mutex<Option<(u64, bool)>>>,
        ask_channel: Arc<agent_kernel::tools::registry::AskChannel>,
        cancel_flag: Arc<std::sync::atomic::AtomicBool>,
        queue: Arc<Mutex<std::collections::VecDeque<String>>>,
        provider_name: String,
        model: String,
        sandbox_id: String,
        auto_yes: bool,
        workspace: std::path::PathBuf,
        permission_mode: String,
        agent_mode: String,
        thinking_level: Option<String>,
        model_catalog: Vec<(String, Vec<String>)>,
    ) -> Self {
        Self {
            cmd_tx,
            ev_rx,
            decision_slot,
            ask_channel,
            cancel_flag,
            queue,
            state: AgentState::Idle,
            composer: Composer::new(),
            modal: None,
            live: Vec::new(),
            active_text: String::new(),
            pending_flush: None,
            turn_started: None,
            queued: Vec::new(),
            usage: None,
            provider_name,
            model,
            sandbox_id,
            auto_yes,
            workspace,
            permission_mode,
            agent_mode,
            thinking_level,
            model_catalog,
            resume_request: None,
            git_info: None,
            git_at: std::time::Instant::now() - std::time::Duration::from_secs(60),
            session_id: 0,
            new_request: false,
            slash_sel: 0,
            slash_dismissed: false,
            todos: Vec::new(),
            spinner: 0,
            quit: false,
            exit_code: 0,
            width: 80,
            pending: Vec::new(),
        }
    }

    pub fn set_width(&mut self, w: u16) {
        self.width = w;
    }

    /// Route one UI intent: mid-turn commands land on the engine's shared
    /// slots (the cmd pump is blocked inside the turn future); turn-boundary
    /// commands go through `cmd_tx` like the desktop frontend.
    pub async fn dispatch(&mut self, cmd: UiCommand) {
        use std::sync::atomic::Ordering;
        self.absorb_display_state(&cmd);
        match cmd {
            UiCommand::ToolDecision {
                request_id,
                approved,
            } => {
                *self.decision_slot.lock().unwrap() = Some((request_id, approved));
            }
            UiCommand::AnswerQuestion { request_id, answer } => {
                self.ask_channel.answer(request_id, answer);
            }
            UiCommand::Cancel => {
                self.cancel_flag.store(true, Ordering::Relaxed);
                self.ask_channel.clear();
                // idle-session Cancel still goes through the pump for the
                // cleanup notice; mid-turn the flag alone is what lands.
                if !self.state.is_active() {
                    let _ = self.cmd_tx.send(UiCommand::Cancel).await;
                }
            }
            UiCommand::Enqueue { text } => {
                self.queue.lock().unwrap().push_back(text);
                let _ = self.cmd_tx.send(UiCommand::DrainQueue).await;
            }
            other => {
                let _ = self.cmd_tx.send(other).await;
            }
        }
    }

    /// 100ms tick: spinner already advanced by main — here we only handle
    /// the deferred stream flush (see `pending_flush`).
    pub fn on_tick(&mut self) {
        if let Some(deadline) = self.pending_flush {
            if std::time::Instant::now() >= deadline {
                self.pending_flush = None;
                self.live.clear();
                self.turn_started = None;
                if !self.active_text.trim().is_empty() {
                    let t = std::mem::take(&mut self.active_text);
                    self.commit(Cell::Agent {
                        md: t,
                        time: crate::events::hm_now(),
                    });
                }
            }
        }
        // Git strip for the bottom status bar — every ~2s, cached. The
        // spawn cost (~10ms) lands on the 100ms tick and is invisible.
        if self.git_at.elapsed().as_secs() >= 2 {
            self.git_at = std::time::Instant::now();
            self.git_info = crate::status::git_info(&self.workspace);
        }
    }

    /// `Set*` commands flow through dispatch from modal resolves and
    /// slash args alike — update the labels shown in border/status so the
    /// TUI never re-parses its own intents.
    pub(crate) fn absorb_display_state(&mut self, cmd: &UiCommand) {
        match cmd {
            UiCommand::SetModel { provider, model } => {
                self.provider_name = provider.clone();
                self.model = model.clone();
            }
            UiCommand::SetPermissionMode { mode } => self.permission_mode = mode.clone(),
            UiCommand::SetAgentMode { mode } => self.agent_mode = mode.clone(),
            UiCommand::SetThinkingLevel { level } => self.thinking_level = Some(level.clone()),
            _ => {}
        }
    }

    /// `/resume` picker result — `main` takes it, rebuilds the actor, then
    /// calls `swap_session` with the fresh channels.
    pub fn take_resume_request(&mut self) -> Option<i64> {
        self.resume_request.take()
    }

    /// `/new` — `main` allocates a fresh id and swaps the actor.
    pub fn take_new_request(&mut self) -> bool {
        std::mem::take(&mut self.new_request)
    }

    /// Point every channel/handle at a NEW actor (spawn or resume) without
    /// discarding the transcript — scrollback is already printed; only the
    /// live plumbing and in-flight state change hands.
    #[allow(clippy::too_many_arguments)]
    pub fn swap_session(
        &mut self,
        cmd_tx: mpsc::Sender<UiCommand>,
        ev_rx: UiReceiver,
        decision_slot: Arc<Mutex<Option<(u64, bool)>>>,
        ask_channel: Arc<agent_kernel::tools::registry::AskChannel>,
        cancel_flag: Arc<std::sync::atomic::AtomicBool>,
        queue: Arc<Mutex<std::collections::VecDeque<String>>>,
    ) {
        self.cmd_tx = cmd_tx;
        self.ev_rx = ev_rx;
        self.decision_slot = decision_slot;
        self.ask_channel = ask_channel;
        self.cancel_flag = cancel_flag;
        self.queue = queue;
        // in-flight state belongs to the old actor — drop it all
        self.state = agent_ipc::AgentState::Idle;
        self.modal = None;
        self.live.clear();
        self.active_text.clear();
        self.turn_started = None;
        self.queued.clear();
        self.pending_flush = None;
    }

    /// Commit one cell → wrapped lines onto the scrollback queue.
    pub fn commit(&mut self, cell: Cell) {
        self.pending.extend(transcript::wrap_lines(
            cell.to_lines(self.width),
            self.width,
        ));
    }

    /// Flush committed lines into terminal scrollback above the viewport.
    pub fn flush_scrollback(&mut self, term: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let lines = std::mem::take(&mut self.pending);
        term.insert_before(lines.len() as u16, |buf| {
            for (i, l) in lines.iter().enumerate() {
                buf.set_line(0, i as u16, l, buf.area.width);
            }
        })
    }

    /// The live region: `live` items rendered in arrival order — text is
    /// plain-wrapped while streaming (markdown reflows on every delta, so
    /// the expensive parse only runs once at commit), reasoning dim+italic,
    /// tools carrying their timer. Bottom-aligned in `height` rows.
    pub fn activity_lines(&mut self, width: usize, height: usize) -> Vec<Line<'static>> {
        use ratatui::style::{Color, Modifier, Style};
        use ratatui::text::Span;
        let dim = Style::default().fg(Color::DarkGray);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let spin = crate::status::SPINNER[self.spinner % crate::status::SPINNER.len()];
        for item in self.live.iter_mut().rev() {
            if lines.len() >= height {
                break;
            }
            match item {
                LiveItem::Text(_) | LiveItem::Reasoning(_) | LiveItem::Delegate(_) => {
                    let (t, style) = match item {
                        LiveItem::Text(t) => (t, Style::default()),
                        LiveItem::Reasoning(t) => (t, dim.add_modifier(Modifier::ITALIC)),
                        LiveItem::Delegate(t) => (t, dim),
                        _ => unreachable!(),
                    };
                    for l in t.rows(width as u16).iter().rev().take(height - lines.len()) {
                        lines.push(Line::from(Span::styled(l.clone(), style)));
                    }
                }
                LiveItem::Tool {
                    name,
                    preview,
                    started,
                    ..
                } => {
                    // Only in-flight tools live here; a finished one has
                    // already been committed to scrollback.
                    let secs = started.elapsed().as_secs_f32();
                    let tool_line = Line::from(vec![
                        Span::styled(format!("{spin} "), Style::default().fg(Color::Cyan)),
                        Span::styled(name.clone(), Style::default().fg(Color::Yellow)),
                        Span::styled(
                            format!("  {}", preview.chars().take(60).collect::<String>()),
                            dim,
                        ),
                        Span::styled(format!("  · {secs:.1}s"), dim),
                    ]);
                    let wrapped = transcript::wrap_lines(vec![tool_line], width as u16);
                    lines.extend(wrapped.into_iter().rev().take(height - lines.len()));
                }
            }
        }
        lines.reverse();
        lines
    }

    // ------------------------------------------------------------------
    // UiEvent → state
    // ------------------------------------------------------------------
}

/// Last few non-empty lines of a failed tool's output — the transcript
/// tail that explains the failure without flooding scrollback.
pub(crate) fn fail_tail(content: &str) -> Vec<String> {
    let nonempty: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    nonempty
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(|l| l.chars().take(100).collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    #[test]
    fn cached_stream_wraps_incrementally_and_reflows_on_resize() {
        for prefix in ["", "  ", "  ↳ "] {
            let mut stream = StreamText::new("", prefix, 12);
            let mut text = String::new();
            for chunk in ["hello", " world", " 中", "文\n", "next\n\n", "tail"] {
                stream.push_str(chunk);
                text.push_str(chunk);
                for width in [12, 8, 20, 12] {
                    let expected: Vec<String> = text
                        .lines()
                        .flat_map(|line| {
                            transcript::wrap_line(
                                &Line::from(Span::raw(format!("{prefix}{line}"))),
                                width as usize,
                            )
                        })
                        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                        .collect();
                    assert_eq!(
                        stream.rows(width),
                        expected,
                        "width={width}, prefix={prefix:?}, text={text:?}"
                    );
                    let cached = stream.rows(width).as_ptr();
                    assert_eq!(stream.rows(width).as_ptr(), cached);
                }
            }
        }
    }
}
