//! `App::update` — the Elm reducer. Every state change goes through here:
//! UI intents route to the active session's actor; tagged kernel events
//! mutate that session's `SessionView` (background sessions only touch
//! sidebar preview/running state — a switched-away turn still progresses).

use agent_ipc::{UiCommand, UiEvent};
use iced::Task;

use super::animation;
use super::message::Message;
use super::state::{
    App, ApprovalRow, MessageRow, Role, SessionView, StepRow, StepState, StreamItem,
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
            Message::Cancel => {
                // Write the session's cancel flag directly — bypasses the
                // command pump so a Cancel isn't queued behind `run_turn`.
                if let Some(mgr) = self.mgr.as_mut() {
                    if let Some(h) = mgr.handle_mut(mgr.active_id) {
                        h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        h.running = false;
                    }
                }
                Task::none()
            }
            Message::Approve(id) => {
                // Echo the pending request_id so the kernel only consumes a
                // verdict meant for THIS approval (P1-a).
                let rid = self.active_view()
                    .and_then(|v| v.pending.as_ref())
                    .map(|p| p.request_id)
                    .unwrap_or(0);
                if let Some(h) = self.mgr.as_ref().and_then(|m| m.active()) {
                    *h.decision.lock().unwrap() = Some((rid, true));
                }
                self.resolve_pending(id, StepState::Success);
                Task::none()
            }
            Message::Deny(id) => {
                let rid = self.active_view()
                    .and_then(|v| v.pending.as_ref())
                    .map(|p| p.request_id)
                    .unwrap_or(0);
                if let Some(h) = self.mgr.as_ref().and_then(|m| m.active()) {
                    *h.decision.lock().unwrap() = Some((rid, false));
                }
                self.resolve_pending(id, StepState::Denied);
                Task::none()
            }
            Message::SelectSession(id) => {
                if let Some(mgr) = self.mgr.as_mut() {
                    mgr.open_session(id);
                    self.active_id = id;
                    self.scroll.reset();
                    // Rebuild the stream from persisted history off-thread —
                    // `view_from_history` markdown-parses every message, which
                    // is an O(history) main-thread stall on big sessions.
                    // Show a loading surface until `SessionLoaded` lands.
                    if !self.views.contains_key(&id) {
                        if let Some(hist) = mgr.store_history(id) {
                            self.loading_session = Some(id);
                            return Task::perform(
                                async move { super::state::view_from_history(&hist) },
                                move |view| Message::SessionLoaded(id, Box::new(view)),
                            );
                        }
                    }
                }
                Task::none()
            }
            // The async history rebuild finished — install the view and
            // scroll to the latest message.
            Message::SessionLoaded(id, view) => {
                if self.loading_session == Some(id) {
                    self.loading_session = None;
                }
                self.views.insert(id, *view);
                self.scroll.snap_to_bottom()
            }
            Message::NewSession => {
                if let Some(mgr) = self.mgr.as_mut() {
                    mgr.new_session();
                    self.active_id = mgr.active_id;
                    self.refresh_sidebar();
                }
                Task::none()
            }
            Message::OpenWorkspace => {
                Task::perform(
                    async {
                        let handle = rfd::AsyncFileDialog::new()
                            .set_title("Open Workspace Directory")
                            .pick_folder()
                            .await;
                        handle.map(|h| h.path().to_path_buf())
                    },
                    |res| match res {
                        Some(path) => Message::WorkspaceSelected(path),
                        None => Message::Noop,
                    },
                )
            }
            Message::WorkspaceSelected(path) | Message::SwitchWorkspace(path) => {
                let hist_to_load = if let Some(mgr) = self.mgr.as_mut() {
                    if let Ok(()) = mgr.switch_workspace(path) {
                        self.workspace_root = mgr.workspace_root.clone();
                        self.workspace_name = self.workspace_root
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .filter(|n| !n.is_empty())
                            .unwrap_or_else(|| self.workspace_root.to_string_lossy().to_string());
                        self.recent_workspaces = mgr.recent_workspaces();
                        self.active_id = mgr.active_id;
                        self.views.clear();
                        let aid = self.active_id;
                        mgr.store_history(aid)
                    } else {
                        None
                    }
                } else {
                    None
                };

                self.refresh_sidebar();
                self.stats.git_branch = super::state::git_branch(&self.workspace_root);
                self.scroll.reset();

                if let Some(hist) = hist_to_load {
                    let aid = self.active_id;
                    self.loading_session = Some(aid);
                    return Task::perform(
                        async move { super::state::view_from_history(&hist) },
                        move |view| Message::SessionLoaded(aid, Box::new(view)),
                    );
                }
                Task::none()
            }
            Message::Noop => Task::none(),
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
                self.scroll.on_scroll(vp);
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
                self.tick_count = self.tick_count.wrapping_add(1);
                let now = std::time::Instant::now();
                // Advance every looping pulse (loading dots + streaming
                // carets) — iced_anim `Animated` values tick forward and
                // flip targets to breathe forever.
                for view in self.views.values_mut() {
                    for p in view.dot_pulses.iter_mut() {
                        animation::pulse_tick(p, now);
                    }
                    for item in view.stream.iter_mut() {
                        if let StreamItem::Message(m) = item {
                            animation::pulse_tick(&mut m.caret, now);
                        }
                    }
                }
                // Drain the tagged kernel queue — route each event to its
                // session's view (active or background).
                let events: Vec<(String, i64, UiEvent)> = self
                    .event_rx
                    .as_ref()
                    .map(|rx| rx.try_iter().collect())
                    .unwrap_or_default();
                let mut tasks = Vec::new();
                for (root, sid, ev) in events {
                    tasks.push(self.apply_session_event(root, sid, ev));
                }
                // Flush the SSE delta buffers ONCE for the frame — deltas
                // accumulated all tick now reparse + land together. Active
                // session snaps to the new bottom; background sessions just
                // keep their view in sync for the next switch.
                let mut landed_active = false;
                for (sid, view) in self.views.iter_mut() {
                    if flush_deltas(view) && *sid == self.active_id {
                        landed_active = true;
                    }
                }
                if landed_active {
                    tasks.push(self.scroll.snap_if_following());
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
            .map(|(id, title, _preview, active, running)| super::state::SessionRow {
                id,
                title,
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
    fn apply_session_event(&mut self, root: String, sid: i64, ev: UiEvent) -> Task<Message> {
        if let Some(mgr) = self.mgr.as_mut() {
            // `handle_mut_at` finds the handle in whichever workspace owns
            // it — a parked workspace's actor keeps streaming after a
            // switch, and its sidebar flags must still update.
            if let Some(h) = mgr.handle_mut_at(&root, sid) {
                match &ev {
                    UiEvent::StateChanged(s) => h.running = s.is_active(),
                    UiEvent::AssistantMessage(t) => {
                        h.preview = t.chars().take(60).collect()
                    }
                    _ => {}
                }
            }
        }
        // A parked workspace's events never land in a view: this shell
        // keys `views` by bare session id, so a foreign event would
        // corrupt a same-numbered session here. Its turn still runs and
        // still updates its sidebar flags (above) — it just can't render
        // until the workspace is switched back and history reloads.
        if root.as_str() != self.workspace_root.to_string_lossy().as_ref() {
            return Task::none();
        }
        // Apply to that session's view (create on demand).
        let view = self.views.entry(sid).or_default();
        apply_to_view(view, &ev);
        // Only the visible session drives the stats bar / scroll pin.
        if sid == self.active_id {
            if let UiEvent::StateChanged(s) = &ev {
                // `label()` not `{:?}` — Debug would dump the whole
                // `diff_summary` into the status bar on approvals.
                self.stats.agent_state = s.label();
            }
            if let UiEvent::Usage { prompt_tokens, completion_tokens, .. } = &ev {
                self.stats.tokens_used = prompt_tokens + completion_tokens;
            }
            Task::none()
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
    // SSE↔frame decoupling: streaming deltas ACCUMULATE into the buffer and
    // flush once per `Tick`; every structural event flushes first so item
    // order stays true (a tool capsule never lands ahead of its prose).
    match ev {
        UiEvent::TextDelta(t) => {
            v.pending_text.push_str(t);
            return;
        }
        UiEvent::ReasoningDelta(t) => {
            v.pending_reasoning.push_str(t);
            return;
        }
        _ => {
            flush_deltas(v);
        }
    }
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
        UiEvent::TextDelta(_) | UiEvent::ReasoningDelta(_) => unreachable!(),
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
                        // If the tool produced a unified diff (fuzzy_patch /
                        // write tools) and the Ask path never stashed one on
                        // the row (e.g. auto-allowed, or approval resolved
                        // before expand), parse it now so the expanded body
                        // shows colored +/- lines instead of raw text.
                        if s.diff_lines.is_empty()
                            && (content.contains("@@") || content.contains("+++"))
                        {
                            // The result text prefixes the diff with a
                            // "patched <path> …" header line — start parsing
                            // at the first real diff marker so that header
                            // isn't rendered as a bogus context row.
                            let body_start = content
                                .lines()
                                .position(|l| {
                                    l.starts_with("@@")
                                        || l.starts_with("--- ")
                                        || l.starts_with("diff ")
                                })
                                .unwrap_or(0);
                            let body: String = content
                                .lines()
                                .skip(body_start)
                                .collect::<Vec<_>>()
                                .join("\n");
                            let parsed = parse_unified_diff(&body);
                            if !parsed.is_empty() {
                                s.diff_lines = parsed;
                            }
                        }
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
        UiEvent::ApprovalRequested { request_id, tool_name, diff, fuzzy } => {
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
                request_id: *request_id,
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

/// Flush the SSE delta buffers into the stream — runs once per `Tick`
/// (and before every structural event) so markdown re-parses O(1/frame)
/// instead of O(1/token). Returns true when content actually landed — the
/// caller uses it to fire exactly one `snap_to_end` per frame.
fn flush_deltas(v: &mut SessionView) -> bool {
    let mut landed = false;
    if !v.pending_reasoning.is_empty() {
        let d = std::mem::take(&mut v.pending_reasoning);
        append_last(v, &d, true);
        landed = true;
    }
    if !v.pending_text.is_empty() {
        let d = std::mem::take(&mut v.pending_text);
        append_last(v, &d, false);
        landed = true;
    }
    landed
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

// `parse_unified_diff` lives in `state.rs` — shared by the live
// `ToolCallFinished`/`ApprovalRequested` path and `view_from_history`'s
// persisted-result replay.
use super::state::parse_unified_diff;
