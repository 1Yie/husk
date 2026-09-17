//! `App::update` — the Elm reducer. Every state change goes through here:
//! UI intents route to the active session's actor; tagged kernel events
//! mutate that session's `SessionView` (background sessions only touch
//! sidebar preview/running state — a switched-away turn still progresses).

use agent_ipc::{UiCommand, UiEvent};
use iced::widget::{operation, scrollable};
#[allow(unused_imports)] use scrollable as _scrollable;
use iced::Task;

use super::message::Message;
use super::state::{
    App, ApprovalRow, DiffKind, DiffLine, MessageRow, Role, SessionView, StepRow,
    StepState, StreamItem,
};

impl App {
    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::InputChanged(s) => {
                self.input = s;
                Task::none()
            }
            Message::Submit => self.submit(false),
            Message::Steer => self.submit(true),
            Message::Approve(id) => {
                if let Some(h) = self.mgr.as_ref().and_then(|m| m.active()) {
                    *h.decision.lock().unwrap() = Some(true);
                }
                self.resolve_pending(id, StepState::Success);
                Task::none()
            }
            Message::Deny(id) => {
                if let Some(h) = self.mgr.as_ref().and_then(|m| m.active()) {
                    *h.decision.lock().unwrap() = Some(false);
                }
                self.resolve_pending(id, StepState::Denied);
                Task::none()
            }
            Message::SelectSession(id) => {
                if let Some(mgr) = self.mgr.as_mut() {
                    mgr.open_session(id);
                    self.active_id = id;
                    self.user_scrolled = false;
                    // Rebuild the stream from the persisted history if this
                    // session's view was never populated (fresh resume).
                    if !self.views.contains_key(&id) {
                        if let Some(hist) = mgr.store_history(id) {
                            self.views.insert(id, super::state::view_from_history(&hist));
                        }
                    }
                }
                operation::snap_to_end(self.stream_scroll.clone())
            }
            Message::NewSession => {
                if let Some(mgr) = self.mgr.as_mut() {
                    mgr.new_session();
                    self.active_id = mgr.active_id;
                    self.refresh_sidebar();
                }
                Task::none()
            }
            // `i` is a stream index — toggle only lands on the right kind.
            Message::ToggleReasoning(i) => {
                if let Some(StreamItem::Message(m)) = self.view_mut().stream.get_mut(i) {
                    m.reasoning_open = !m.reasoning_open;
                }
                Task::none()
            }
            Message::ToggleStep(i) => {
                if let Some(StreamItem::Tool(s)) = self.view_mut().stream.get_mut(i) {
                    s.expanded = !s.expanded;
                }
                Task::none()
            }
            Message::Scrolled(vp) => {
                self.user_scrolled = vp.relative_offset().y < 0.98;
                Task::none()
            }
            Message::LinkClicked(uri) => {
                let _ = open::that_detached(uri.as_str());
                Task::none()
            }
            Message::WindowReady(id) => {
                self.window_id = id;
                Task::none()
            }
            Message::WindowAction(action) => {
                let Some(id) = self.window_id else { return Task::none() };
                use super::message::WinAction;
                match action {
                    WinAction::Drag => iced::window::drag(id),
                    WinAction::Minimize => iced::window::minimize(id, true),
                    WinAction::ToggleMaximize => iced::window::toggle_maximize(id),
                    WinAction::Close => iced::window::close(id),
                }
            }
            Message::CopyMessage(i) => {
                // Copy the message's raw markdown source to the clipboard.
                let text = self
                    .active_view()
                    .and_then(|v| match v.stream.get(i) {
                        Some(StreamItem::Message(m)) => Some(m.text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if text.is_empty() {
                    Task::none()
                } else {
                    iced::clipboard::write(text)
                }
            }
            Message::ToggleSelect(i) => {
                if let Some(StreamItem::Message(m)) = self.view_mut().stream.get_mut(i) {
                    m.selectable = !m.selectable;
                    if m.selectable {
                        // Refresh the editor so it shows the latest text.
                        m.editor = iced::widget::text_editor::Content::with_text(&m.text);
                    }
                }
                Task::none()
            }
            Message::SelectAction(i, action) => {
                // Read-only editor — swallow Edit actions so the text can't
                // change, keep Select/Move/Click/Drag/Copy for selection.
                if action.is_edit() {
                    return Task::none();
                }
                if let Some(StreamItem::Message(m)) = self.view_mut().stream.get_mut(i) {
                    m.editor.perform(action);
                }
                Task::none()
            }
            Message::Tick => {
                self.tick = self.tick.wrapping_add(1);
                // Drain the tagged kernel queue — route each event to its
                // session's view (active or background).
                let events: Vec<(i64, UiEvent)> = self
                    .mgr
                    .as_ref()
                    .map(|m| m.event_rx.try_iter().collect())
                    .unwrap_or_default();
                let mut tasks = Vec::new();
                for (sid, ev) in events {
                    tasks.push(self.apply_session_event(sid, ev));
                }
                // Refresh sidebar previews (running markers update live).
                self.refresh_sidebar();
                if tasks.is_empty() {
                    Task::none()
                } else {
                    Task::batch(tasks)
                }
            }
        }
    }

    /// Rebuild sidebar rows from the manager's persisted + live state.
    fn refresh_sidebar(&mut self) {
        let Some(mgr) = &self.mgr else { return };
        self.sessions = mgr
            .sidebar_rows()
            .into_iter()
            .map(|(id, title, preview, active, running)| super::state::SessionRow {
                id,
                title,
                preview: if running { "⟳ running…".into() } else { preview },
                active,
                running,
                timestamp: String::new(),
            })
            .collect();
    }

    /// Submit or steer the composer text to the ACTIVE session's actor.
    fn submit(&mut self, steer: bool) -> Task<Message> {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return Task::none();
        }
        self.input.clear();
        let Some(h) = self.mgr.as_ref().and_then(|m| m.active()) else {
            return Task::none();
        };
        if steer {
            let t = text.clone();
            if h.steer_tx.try_send(t).is_err() {
                let _ = h.cmd_tx.try_send(UiCommand::Steer { text });
            }
        } else {
            let _ = h.cmd_tx.try_send(UiCommand::Prompt { text });
        }
        Task::none()
    }

    /// Route one tagged `UiEvent` to its session's `SessionView`.
    fn apply_session_event(&mut self, sid: i64, ev: UiEvent) -> Task<Message> {
        // Update the live handle's running/preview for the sidebar.
        if let Some(mgr) = self.mgr.as_mut() {
            if let Some(h) = mgr.handle_mut(sid) {
                match &ev {
                    UiEvent::StateChanged(s) => h.running = s.is_active(),
                    UiEvent::AssistantMessage(t) => {
                        h.preview = t.chars().take(60).collect()
                    }
                    _ => {}
                }
            }
        }
        // Apply to that session's view (create on demand).
        let view = self.views.entry(sid).or_default();
        apply_to_view(view, &ev);
        // Only the visible session drives the stats bar / scroll pin.
        if sid == self.active_id {
            if let UiEvent::StateChanged(s) = &ev {
                self.stats.agent_state = format!("{s:?}");
            }
            if let UiEvent::Usage { prompt_tokens, completion_tokens } = &ev {
                self.stats.tokens_used = prompt_tokens + completion_tokens;
            }
            if self.user_scrolled {
                Task::none()
            } else {
                operation::snap_to_end(self.stream_scroll.clone())
            }
        } else {
            Task::none()
        }
    }

    /// Resolve the pending approval + push the awaiting step to a final state.
    fn resolve_pending(&mut self, _step_id: usize, final_state: StepState) {
        let v = self.view_mut();
        v.pending = None;
        for item in v.stream.iter_mut().rev() {
            if let StreamItem::Tool(s) = item {
                if s.state == StepState::AwaitingConfirm {
                    s.state = final_state;
                    break;
                }
            }
        }
    }
}

/// Apply a `UiEvent` to one session's view — the shared per-session logic.
fn apply_to_view(v: &mut SessionView, ev: &UiEvent) {
    match ev {
        UiEvent::StateChanged(s) => {
            v.is_active = s.is_active();
        }
        UiEvent::UserPrompt(text) => {
            v.stream.push(StreamItem::Message(MessageRow::new(
                v.stream.len(),
                Role::User,
                text.clone(),
            )));
        }
        UiEvent::TextDelta(t) => append_last(v, t, false),
        UiEvent::ReasoningDelta(t) => append_last(v, t, true),
        UiEvent::AssistantMessage(_) => {
            if let Some(StreamItem::Message(m)) = v.stream.last_mut() {
                m.streaming = false;
                m.thinking_done = true;
                m.reasoning_open = false;
            }
        }
        // A tool call lands INLINE at this point in the stream — the chain
        // shows what ran, where, in order (Codex-style), not a bottom strip.
        UiEvent::ToolCallStarted { name, args_preview } => {
            v.stream.push(StreamItem::Tool(StepRow {
                id: v.stream.len(),
                name: name.clone(),
                state: StepState::Running,
                detail: args_preview.clone(),
                output: String::new(),
                diff_lines: vec![],
                expanded: false,
            }));
        }
        UiEvent::ToolCallFinished { name, ok, content, .. } => {
            if let Some(p) = &v.pending {
                if p.tool_name == *name {
                    v.pending = None;
                }
            }
            for item in v.stream.iter_mut().rev() {
                if let StreamItem::Tool(s) = item {
                    if s.name == *name
                        && matches!(s.state, StepState::Running | StepState::AwaitingConfirm)
                    {
                        s.state = if *ok { StepState::Success } else { StepState::Error };
                        // Full output → expandable body, not the one-liner.
                        s.output = content.clone();
                        // No args preview? fall back to a flat one-line
                        // output summary (newlines collapsed, no wrapping).
                        if s.detail.is_empty() {
                            s.detail = content
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(60)
                                .collect();
                        }
                        break;
                    }
                }
            }
        }
        UiEvent::ApprovalRequested { tool_name, diff, fuzzy } => {
            let mut sid = v.stream.len().saturating_sub(1);
            for (i, item) in v.stream.iter_mut().enumerate().rev() {
                if let StreamItem::Tool(s) = item {
                    if s.state == StepState::Running {
                        s.state = StepState::AwaitingConfirm;
                        sid = i;
                        break;
                    }
                }
            }
            let diff_lines = parse_unified_diff(diff);
            v.pending = Some(ApprovalRow {
                step_id: sid,
                tool_name: tool_name.clone(),
                title: format!("Approve {tool_name}"),
                diff_text: diff.clone(),
                fuzzy: *fuzzy,
                risk: "normal".into(),
            });
            // Stash the parsed diff on the awaiting step's row so the
            // expanded body still renders it after approve/deny resolves.
            if let Some(StreamItem::Tool(s)) = v.stream.get_mut(sid) {
                s.diff_lines = diff_lines.clone();
            }
            v.pending_diff = diff_lines;
        }
        UiEvent::Usage { .. } => {} // surfaced on the active session's stats
        UiEvent::SystemMessage(msg) | UiEvent::Error(msg) => {
            v.stream.push(StreamItem::Message(MessageRow::new(
                v.stream.len(),
                Role::System,
                msg.clone(),
            )));
        }
    }
}

/// Append a delta to the streaming agent row — the LAST stream item if it's
/// a streaming agent message, else a fresh message appended after any tool
/// capsules (so post-tool text starts a new bubble, not merged into the
/// pre-tool one).
fn append_last(v: &mut SessionView, delta: &str, reasoning: bool) {
    let last_is_streaming = matches!(
        v.stream.last(),
        Some(StreamItem::Message(m)) if m.role == Role::Agent && m.streaming
    );
    if !last_is_streaming {
        let mut m = MessageRow::new(v.stream.len(), Role::Agent, String::new());
        m.streaming = true;
        v.stream.push(StreamItem::Message(m));
    }
    if let Some(StreamItem::Message(row)) = v.stream.last_mut() {
        if reasoning {
            row.reasoning.push_str(delta);
        } else {
            row.text.push_str(delta);
            row.reparse(); // keep markdown items in sync for the next frame
        }
        row.streaming = true;
    }
}

/// Split a unified diff into `DiffLine` rows.
fn parse_unified_diff(diff: &str) -> Vec<DiffLine> {
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
