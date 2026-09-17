//! `App::update` — the Elm reducer. Every state change goes through here:
//! UI intents send kernel commands, `KernelEvent` mutates the flat row vecs.

use agent_ipc::{UiCommand, UiEvent};
use iced::widget::{operation, scrollable};
#[allow(unused_imports)]
use scrollable as _scrollable_unused;
use iced::Task;

use super::message::Message;
use super::state::{App, ApprovalRow, DiffKind, DiffLine, MessageRow, Role, StepRow, StepState};

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
                *self.decision.lock().unwrap() = Some(true);
                self.resolve_pending(id, StepState::Success);
                Task::none()
            }
            Message::Deny(id) => {
                *self.decision.lock().unwrap() = Some(false);
                self.resolve_pending(id, StepState::Denied);
                Task::none()
            }
            Message::SelectSession(_id) => Task::none(), // single-session stub
            Message::NewSession => Task::none(),       // multi-session not wired
            Message::ToggleReasoning(i) => {
                if let Some(m) = self.messages.get_mut(i) {
                    m.reasoning_open = !m.reasoning_open;
                }
                Task::none()
            }
            Message::ToggleStep(i) => {
                if let Some(s) = self.active_steps.get_mut(i) {
                    s.expanded = !s.expanded;
                }
                Task::none()
            }
            Message::Tick => {
                self.tick = self.tick.wrapping_add(1);
                // Drain any kernel events that piled up between frames.
                if let Some(rx) = &self.event_rx {
                    let pending: Vec<UiEvent> = rx.lock().unwrap().try_iter().collect();
                    // Process them inline (recursion-free, one update pass).
                    for ev in pending {
                        // Reuse apply_event but it returns Task — collect and
                        // batch. For simplicity apply directly here.
                        let _ = self.apply_event(ev);
                    }
                }
                Task::none()
            }
            Message::Scrolled(vp) => {
                // If the user scrolled up off the bottom, stop auto-pinning.
                self.user_scrolled = vp.relative_offset().y < 0.98;
                Task::none()
            }
        }
    }

    /// Submit or steer the composer text.
    fn submit(&mut self, steer: bool) -> Task<Message> {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return Task::none();
        }
        self.input.clear();
        if steer {
            if let Some(tx) = &self.steer_tx {
                let t = text.clone();
                if tx.try_send(t).is_err() {
                    if let Some(cmd) = &self.cmd_tx {
                        let _ = cmd.try_send(UiCommand::Steer { text });
                    }
                }
            }
        } else if let Some(cmd) = &self.cmd_tx {
            let _ = cmd.try_send(UiCommand::Prompt { text });
        }
        Task::none()
    }

    /// One `UiEvent` → `App` mutation. The kernel-side `apply_event` logic
    /// from the Slint bridge, as an `App` method.
    pub fn apply_event(&mut self, ev: UiEvent) -> Task<Message> {
        match ev {
            UiEvent::StateChanged(s) => {
                self.stats.agent_state = format!("{s:?}");
                self.is_active = s.is_active();
            }
            UiEvent::UserPrompt(text) => {
                self.messages.push(MessageRow {
                    id: self.messages.len(),
                    role: Role::User,
                    text,
                    reasoning: String::new(),
                    reasoning_open: false,
                    streaming: false,
                });
            }
            UiEvent::TextDelta(t) => self.append_last(&t, false),
            UiEvent::ReasoningDelta(t) => self.append_last(&t, true),
            UiEvent::AssistantMessage(_) => {
                if let Some(last) = self.messages.last_mut() {
                    last.streaming = false;
                }
            }
            UiEvent::ToolCallStarted { name } => {
                self.active_steps.push(StepRow {
                    id: self.active_steps.len(),
                    name,
                    state: StepState::Running,
                    detail: String::new(),
                    expanded: false,
                });
            }
            UiEvent::ToolCallFinished { name, ok, content, .. } => {
                if let Some(p) = &self.pending {
                    if p.tool_name == name {
                        self.pending = None;
                    }
                }
                for s in self.active_steps.iter_mut().rev() {
                    if s.name == name
                        && matches!(s.state, StepState::Running | StepState::AwaitingConfirm)
                    {
                        s.state = if ok { StepState::Success } else { StepState::Error };
                        s.detail = content.chars().take(60).collect();
                        break;
                    }
                }
            }
            UiEvent::ApprovalRequested { tool_name, diff, fuzzy } => {
                for s in self.active_steps.iter_mut().rev() {
                    if s.state == StepState::Running {
                        s.state = StepState::AwaitingConfirm;
                        break;
                    }
                }
                self.pending = Some(ApprovalRow {
                    step_id: self.active_steps.len().saturating_sub(1),
                    tool_name: tool_name.clone(),
                    title: format!("Approve {tool_name}"),
                    diff_text: diff.clone(),
                    fuzzy,
                    risk: "normal".into(),
                });
                self.pending_diff = parse_unified_diff(&diff);
            }
            UiEvent::Usage { prompt_tokens, completion_tokens } => {
                self.stats.tokens_used = prompt_tokens + completion_tokens;
            }
            UiEvent::SystemMessage(msg) | UiEvent::Error(msg) => {
                self.messages.push(MessageRow {
                    id: self.messages.len(),
                    role: Role::System,
                    text: msg,
                    reasoning: String::new(),
                    reasoning_open: false,
                    streaming: false,
                });
            }
        }
        // Pin to bottom unless the user scrolled up.
        if self.user_scrolled {
            Task::none()
        } else {
            operation::snap_to_end(self.stream_scroll.clone())
        }
    }

    /// Append a delta to the streaming agent row — the last row only if it's
    /// an agent row still streaming; else push a fresh one.
    fn append_last(&mut self, delta: &str, reasoning: bool) {
        let target = self
            .messages
            .last()
            .filter(|m| m.role == Role::Agent && m.streaming)
            .map(|_| self.messages.len() - 1);
        let i = match target {
            Some(i) => i,
            None => {
                self.messages.push(MessageRow {
                    id: self.messages.len(),
                    role: Role::Agent,
                    text: String::new(),
                    reasoning: String::new(),
                    reasoning_open: false,
                    streaming: true,
                });
                self.messages.len() - 1
            }
        };
        let row = &mut self.messages[i];
        if reasoning {
            row.reasoning.push_str(delta);
        } else {
            row.text.push_str(delta);
        }
        row.streaming = true;
    }

    /// Resolve the pending approval + push the awaiting step to a final state.
    fn resolve_pending(&mut self, _step_id: usize, final_state: StepState) {
        self.pending = None;
        for s in self.active_steps.iter_mut().rev() {
            if s.state == StepState::AwaitingConfirm {
                s.state = final_state;
                break;
            }
        }
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
        let (kind, o, n) = if let Some(rest) = line.strip_prefix('+') {
            let v = (DiffKind::Add, -1, new_ln);
            new_ln += 1;
            let _ = rest;
            v
        } else if let Some(rest) = line.strip_prefix('-') {
            let v = (DiffKind::Delete, old_ln, -1);
            old_ln += 1;
            let _ = rest;
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
