//! Kernel event + terminal input handling for `App`. Owns the live-item
//! stream bookkeeping, modal routing, and the keymap — every path returns
//! `UiCommand`s so `dispatch` stays the single routing chokepoint.

use agent_ipc::{AgentState, UiCommand, UiEvent};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{fail_tail, App, ChildTool, LiveItem, StreamText, TodoItem};
use crate::cells::Cell;
use crate::modals::{Modal, ModalOutcome};

impl App {
    pub fn on_ui_event(&mut self, ev: UiEvent) {
        match ev {
            UiEvent::UserPrompt(t) => self.commit(Cell::User {
                text: t,
                time: hm_now(),
            }),
            UiEvent::TextDelta { text, parent } => {
                if parent.is_some() {
                    match self.live.last_mut() {
                        Some(LiveItem::Delegate(buf)) => buf.push_str(&text),
                        _ => self.live.push(LiveItem::Delegate(StreamText::new(
                            &text, "  ↳ ", self.width,
                        ))),
                    }
                } else {
                    self.active_text.push_str(&text);
                    match self.live.last_mut() {
                        Some(LiveItem::Text(buf)) => buf.push_str(&text),
                        _ => self
                            .live
                            .push(LiveItem::Text(StreamText::new(&text, "", self.width))),
                    }
                }
            }
            UiEvent::ReasoningDelta { text, parent } => {
                if parent.is_none() {
                    match self.live.last_mut() {
                        Some(LiveItem::Reasoning(buf)) => buf.push_str(&text),
                        _ => self.live.push(LiveItem::Reasoning(StreamText::new(
                            &text, "  ", self.width,
                        ))),
                    }
                }
            }
            UiEvent::ToolCallStarted {
                name,
                args_preview,
                parent,
                ..
            } => {
                // Batch children are not shown live: they are collected and
                // committed under their parent once it finishes.
                match parent {
                    None => {
                        self.flush_mid_text();
                        self.live.push(LiveItem::Tool {
                            name,
                            preview: args_preview,
                            started: std::time::Instant::now(),
                            children: Vec::new(),
                        });
                    }
                    Some(parent_name) => {
                        if let Some(LiveItem::Tool { children, .. }) = self.parent_tool(&parent_name) {
                            children.push(ChildTool {
                                name,
                                preview: args_preview,
                                done: false,
                                ok: false,
                                ui_type: None,
                                tail: Vec::new(),
                            });
                        }
                    }
                }
            }
            UiEvent::ToolCallFinished {
                name,
                ok,
                content,
                parent,
                ui_type,
                ..
            } => {
                // `todo` results carry the whole list — keep it as the
                // persistent panel state; `clear` reports no list and
                // empties the panel.
                if ok && (name == "todo" || ui_type.as_deref() == Some("todo")) {
                    self.todos = parse_todos(&content);
                }
                let tail = if ok { Vec::new() } else { fail_tail(&content) };
                if let Some(parent_name) = parent {
                    // Fill the first unfinished child of this name — children
                    // run concurrently, so completion order is not start order.
                    if let Some(LiveItem::Tool { children, .. }) = self.parent_tool(&parent_name) {
                        if let Some(child) = children
                            .iter_mut()
                            .find(|c| !c.done && c.name == name)
                        {
                            child.done = true;
                            child.ok = ok;
                            child.ui_type = ui_type;
                            child.tail = tail;
                        }
                    }
                    return;
                }
                // Top-level: take its live entry out (it commits exactly once
                // here) and carry its batch children along.
                let idx = self
                    .live
                    .iter()
                    .rposition(|it| matches!(it, LiveItem::Tool { name: n, .. } if *n == name));
                let (preview, secs, children) = match idx.map(|i| self.live.remove(i)) {
                    Some(LiveItem::Tool { preview, started, children, .. }) => {
                        (preview, Some(started.elapsed().as_secs_f32()), children)
                    }
                    _ => (String::new(), None, Vec::new()),
                };
                let is_batch = !children.is_empty();
                self.commit(Cell::Tool {
                    name,
                    preview,
                    ok,
                    tail,
                    secs,
                });
                if is_batch {
                    for child in children {
                        self.commit(Cell::Child(child));
                    }
                }
            }
            UiEvent::ApprovalRequested {
                request_id,
                tool_name,
                diff,
                fuzzy,
            } => {
                if self.auto_yes {
                    *self.decision_slot.lock().unwrap() = Some((request_id, true));
                    self.commit(Cell::Note(format!("auto-approved {tool_name}")));
                } else {
                    self.modal = Some(Modal::Approval {
                        request_id,
                        tool_name,
                        diff,
                        fuzzy,
                    });
                }
            }
            UiEvent::QuestionAsked {
                request_id,
                question,
                options,
            } => {
                if self.auto_yes {
                    let answer = options.first().map(|o| o.label.clone()).unwrap_or_default();
                    self.ask_channel.answer(request_id, answer);
                    self.commit(Cell::Note(format!("auto-answered: {question}")));
                } else {
                    self.modal = Some(Modal::Question {
                        request_id,
                        question,
                        options,
                        selected: 0,
                        free: String::new(),
                    });
                }
            }
            UiEvent::QuestionAnswered { request_id } => {
                if matches!(&self.modal, Some(Modal::Question { request_id: r, .. }) if *r == request_id)
                {
                    self.modal = None;
                }
            }
            UiEvent::AssistantMessage(m) => {
                self.pending_flush = None;
                let elapsed = self
                    .turn_started
                    .take()
                    .map(|t0| t0.elapsed().as_secs_f32());
                self.active_text.clear();
                self.live.clear();
                self.commit(Cell::Agent {
                    md: m,
                    time: hm_now(),
                });
                if let Some(secs) = elapsed {
                    self.commit(Cell::Worked { secs });
                }
            }
            UiEvent::SystemMessage(m) => self.commit(Cell::Sys(m)),
            UiEvent::Error(e) => self.commit(Cell::Err(e)),
            UiEvent::StateChanged(s) => {
                let was_active = self.state.is_active();
                self.state = s;
                if self.state.is_active() && !was_active {
                    self.turn_started = Some(std::time::Instant::now());
                }
                if !self.state.is_active() {
                    // Turn ended — drop the parked modal and arm the
                    // deferred flush: a successful finish commits via the
                    // AssistantMessage that follows microseconds later; a
                    // cancel/fail path has no message, so the deadline
                    // commits whatever text streamed in. `live` stays until
                    // the flush/commit so nothing pops mid-frame.
                    if let Some(m) = self.modal.take() {
                        let _ = m;
                    }
                    if !self.active_text.trim().is_empty() && was_active {
                        self.pending_flush =
                            Some(std::time::Instant::now() + std::time::Duration::from_millis(150));
                    } else {
                        self.live.clear();
                        self.turn_started = None;
                    }
                    if matches!(self.state, AgentState::Failed(_)) {
                        self.exit_code = 1;
                    }
                }
            }
            UiEvent::Usage {
                prompt_tokens,
                completion_tokens,
                context_window,
                ..
            } => {
                self.usage = Some((
                    prompt_tokens + completion_tokens,
                    prompt_tokens + completion_tokens,
                    context_window,
                ));
            }
            UiEvent::QueuedPrompts { items } => self.queued = items,
            UiEvent::Custom { kind, payload } => {
                let text = |key: &str| payload[key].as_str().unwrap_or_default().to_string();
                match kind.as_str() {
                    "plan_submitted" => self.commit(Cell::Plan(text("plan"))),
                    "blueprint_report" => self.commit(if payload["errors"].as_u64() > Some(0) {
                        Cell::Err(text("text"))
                    } else {
                        Cell::Sys(text("text"))
                    }),
                    _ => {}
                }
            }
            UiEvent::SessionToolWhitelisted { tool_name, stopped } => {
                self.commit(Cell::Note(if stopped {
                    format!("{tool_name} 移出本会话白名单")
                } else {
                    format!("{tool_name} 已加入本会话白名单")
                }));
            }
            UiEvent::TurnRetry => self.commit(Cell::Note("重试上一回合".into())),
            UiEvent::CompactionStarted { manual } => {
                self.commit(Cell::Note(if manual {
                    "手动压缩上下文…".into()
                } else {
                    "自动压缩上下文…".into()
                }));
            }
            UiEvent::Compacted {
                before_tokens,
                after_tokens,
                removed_messages,
                ..
            } => self.commit(Cell::Note(format!(
                "压缩完成 {before_tokens}→{after_tokens} tok（合并 {removed_messages} 条）"
            ))),
        }
    }

    // ------------------------------------------------------------------
    // terminal event → actions
    // ------------------------------------------------------------------

    /// Returns commands to send (kept sync — caller awaits the mpsc send).
    /// Commit narration text accumulated so far — runs at every tool-call
    /// boundary so the transcript reads text → tool → text in true ReAct
    /// order instead of all tools first and the whole reply last.
    pub(crate) fn flush_mid_text(&mut self) {
        self.flush_reasoning();
        if self.active_text.trim().is_empty() {
            return;
        }
        let t = std::mem::take(&mut self.active_text);
        self.commit(Cell::Agent {
            md: t,
            time: hm_now(),
        });
        self.live.retain(|i| !matches!(i, LiveItem::Text(_)));
    }

    /// The in-flight parent a batch child belongs to (most recent match).
    fn parent_tool(&mut self, parent_name: &str) -> Option<&mut LiveItem> {
        self.live
            .iter_mut()
            .rev()
            .find(|it| matches!(it, LiveItem::Tool { name, .. } if name == parent_name))
    }

    /// Commit the reasoning accumulated before a tool boundary, so it sits
    /// in scrollback right before the call it led to.
    fn flush_reasoning(&mut self) {
        let mut kept = Vec::with_capacity(self.live.len());
        for item in std::mem::take(&mut self.live) {
            match item {
                LiveItem::Reasoning(buf) => {
                    let text = buf.text().to_string();
                    if !text.trim().is_empty() {
                        self.commit(Cell::Reasoning(text));
                    }
                }
                other => kept.push(other),
            }
        }
        self.live = kept;
    }

    pub fn on_term_event(&mut self, ev: Event) -> Vec<UiCommand> {
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let before = self.composer.text();
                let out = self.on_key(key);
                // Editing re-opens a dismissed panel and resets its cursor.
                if self.composer.text() != before {
                    self.slash_sel = 0;
                    self.slash_dismissed = false;
                }
                out
            }
            Event::Paste(text) => {
                if self.modal.is_none() {
                    self.composer.insert_text(&text);
                }
                Vec::new()
            }
            Event::Resize(w, _h) => {
                self.width = w;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Vec<UiCommand> {
        // Modal owns all keys while open.
        if let Some(m) = &mut self.modal {
            let model_stage = match m {
                Modal::Models { stage, .. } => Some(*stage),
                _ => None,
            };
            match m.handle_key(key) {
                ModalOutcome::Send(cmds) => {
                    // transcript echo for the resolution
                    if let Some(Modal::Approval { tool_name, .. }) = &self.modal {
                        let approved = cmds
                            .iter()
                            .any(|c| matches!(c, UiCommand::ToolDecision { approved: true, .. }));
                        let name = tool_name.clone();
                        self.commit(Cell::Note(format!(
                            "{} {name}",
                            if approved { "✓ 批准" } else { "✗ 拒绝" }
                        )));
                    }
                    self.modal = None;
                    return cmds;
                }
                ModalOutcome::Resume(id) => {
                    self.resume_request = Some(id);
                    self.modal = None;
                    return Vec::new();
                }
                ModalOutcome::Close => {
                    self.modal = None;
                    return Vec::new();
                }
                ModalOutcome::Handled => {
                    // Refill only when changing stages, preserving navigation.
                    if let Some(Modal::Models {
                        stage,
                        provider,
                        items,
                        selected,
                    }) = &mut self.modal
                    {
                        if model_stage != Some(*stage) {
                            let current = if *stage == 0 {
                                *items =
                                    self.model_catalog.iter().map(|(p, _)| p.clone()).collect();
                                provider.as_str()
                            } else {
                                *items = self
                                    .model_catalog
                                    .iter()
                                    .find(|(p, _)| p == provider)
                                    .map(|(_, ms)| ms.clone())
                                    .unwrap_or_default();
                                if *provider == self.provider_name {
                                    self.model.as_str()
                                } else {
                                    ""
                                }
                            };
                            *selected = items.iter().position(|m| m == current).unwrap_or(0);
                        }
                    }
                    return Vec::new();
                }
            }
        }

        // `/` autocomplete — while the panel is up, Tab fills the selected
        // candidate, ↑/↓ move within it (instead of history/composer nav),
        // Esc dismisses the panel, Enter fills unless the typed text is
        // already a complete command (then it falls through to submit).
        let cands = self.slash_candidates();
        if !cands.is_empty() {
            self.slash_sel = self.slash_sel.min(cands.len() - 1);
            match key.code {
                KeyCode::Tab => {
                    let (name, _) = cands[self.slash_sel];
                    self.composer.set_text(name);
                    self.slash_dismissed = true;
                    return Vec::new();
                }
                KeyCode::Up => {
                    self.slash_sel = if self.slash_sel == 0 {
                        cands.len() - 1
                    } else {
                        self.slash_sel - 1
                    };
                    return Vec::new();
                }
                KeyCode::Down => {
                    self.slash_sel = (self.slash_sel + 1) % cands.len();
                    return Vec::new();
                }
                KeyCode::Esc => {
                    self.slash_dismissed = true;
                    return Vec::new();
                }
                KeyCode::Enter => {
                    let typed = self.composer.text();
                    let exact = cands.iter().any(|(n, _)| *n == typed);
                    if !exact {
                        let (name, _) = cands[self.slash_sel];
                        self.composer.set_text(name);
                        self.slash_dismissed = true;
                        return Vec::new();
                    }
                    // exact → fall through, the Enter arm submits
                }
                _ => {}
            }
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match (key.code, ctrl, alt) {
            (KeyCode::Char('c'), true, _) => {
                if self.state.is_active() {
                    vec![UiCommand::Cancel]
                } else {
                    self.quit = true;
                    Vec::new()
                }
            }
            (KeyCode::Char('d'), true, _) => {
                self.quit = true;
                Vec::new()
            }
            (KeyCode::Esc, ..) if self.state.is_active() => vec![UiCommand::Cancel],
            (KeyCode::Enter, false, false) => {
                let text = self.composer.take_submit();
                let t = text.trim();
                if t.is_empty() {
                    return Vec::new();
                }
                if let Some(cmds) = self.slash(t) {
                    // Kernel-forwarded `/x` echoes `UserPrompt` back on its
                    // own; locally-handled commands need their echo here.
                    let forwarded = cmds.iter().any(|c| matches!(c, UiCommand::Prompt { .. }));
                    if !forwarded {
                        self.commit(Cell::User {
                            text: t.to_string(),
                            time: hm_now(),
                        });
                    }
                    return cmds;
                }
                // mid-turn submissions park in the kernel queue
                if self.state.is_active() {
                    vec![UiCommand::Enqueue {
                        text: t.to_string(),
                    }]
                } else {
                    vec![UiCommand::Prompt { text }]
                }
            }
            (KeyCode::Enter, ..) => {
                // Ctrl+Enter / Alt+Enter / Shift+Enter(when terminal reports it)
                self.composer.newline();
                Vec::new()
            }
            (KeyCode::Char('j'), true, _) => {
                self.composer.newline();
                Vec::new()
            }
            (KeyCode::Char('w'), true, _) | (KeyCode::Backspace, _, true) => {
                self.composer.delete_word_back();
                Vec::new()
            }
            (KeyCode::Char('u'), true, _) => {
                self.composer.clear();
                Vec::new()
            }
            (KeyCode::Char('a'), true, _) => {
                self.composer.home();
                Vec::new()
            }
            (KeyCode::Char('e'), true, _) => {
                self.composer.end();
                Vec::new()
            }
            (KeyCode::Char(c), _, _) => {
                self.composer.insert_char(c);
                Vec::new()
            }
            (KeyCode::Backspace, ..) => {
                self.composer.backspace();
                Vec::new()
            }
            (KeyCode::Delete, ..) => {
                self.composer.delete();
                Vec::new()
            }
            (KeyCode::Left, ..) => {
                self.composer.move_left();
                Vec::new()
            }
            (KeyCode::Right, ..) => {
                self.composer.move_right();
                Vec::new()
            }
            (KeyCode::Up, ..) => {
                self.composer.move_up();
                Vec::new()
            }
            (KeyCode::Down, ..) => {
                self.composer.move_down();
                Vec::new()
            }
            (KeyCode::Home, ..) => {
                self.composer.home();
                Vec::new()
            }
            (KeyCode::End, ..) => {
                self.composer.end();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// Parse the `todo` tool's rendered output:
/// `{done}/{total} done` header then `[x] #id text` / `[ ] #id text` rows.
/// Anything unparseable (e.g. `Cleared n todos.`) yields an empty panel.
pub(crate) fn parse_todos(content: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    let mut in_list = false;
    for line in content.lines() {
        let l = line.trim();
        if !in_list {
            // `N/M done` header marks where the checklist starts
            if let Some((a, b)) = l.split_once('/') {
                if b.ends_with(" done") && a.trim().parse::<u32>().is_ok() {
                    in_list = true;
                }
            }
            continue;
        }
        let (done, rest) = if let Some(r) = l.strip_prefix("[x]") {
            (true, r.trim())
        } else if let Some(r) = l.strip_prefix("[ ]") {
            (false, r.trim())
        } else {
            continue;
        };
        let rest = rest.strip_prefix('#').unwrap_or(rest);
        if let Some((id_s, text)) = rest.split_once(' ') {
            if let Ok(id) = id_s.trim().parse::<u32>() {
                items.push(TodoItem {
                    id,
                    text: text.trim().to_string(),
                    done,
                });
            }
        }
    }
    items
}

/// `5:33 AM` — libc localtime (no chrono in the dep tree).
pub fn hm_now() -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        let (h24, m) = (tm.tm_hour, tm.tm_min);
        let (h12, ap) = match h24 {
            0 => (12, "AM"),
            h if h < 12 => (h, "AM"),
            12 => (12, "PM"),
            h => (h - 12, "PM"),
        };
        format!("{h12}:{m:02} {ap}")
    }
}
