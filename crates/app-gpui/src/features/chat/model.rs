use agent_ipc::UiEvent;
use agent_ipc::events::{ApprovalAction, AskOption};
use agent_llm::types::{ChatMessage, NoticeKind, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    User,
    Assistant,
    Reasoning,
    Tool,
    Notice,
    Error,
    Child,
}

impl EntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "你",
            Self::Assistant => "Husk",
            Self::Reasoning => "思考过程",
            Self::Tool => "工具调用",
            Self::Notice => "提示",
            Self::Error => "错误",
            Self::Child => "子任务",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub kind: EntryKind,
    pub title: String,
    pub text: String,
    pub detail: String,
    pub complete: bool,
    pub ok: Option<bool>,
    pub timestamp: Option<i64>,
    pub duration_ms: Option<i64>,
    child_id: Option<u64>,
    parent: Option<String>,
    tool_call_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Approval {
    pub request_id: u64,
    pub tool_name: String,
    pub diff: String,
    pub fuzzy: bool,
    pub actions: Vec<ApprovalAction>,
    pub child_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Question {
    pub request_id: u64,
    pub text: String,
    pub options: Vec<AskOption>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub context_window: u32,
    pub cached_tokens: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ChatModel {
    pub entries: Vec<Entry>,
    pub approvals: Vec<Approval>,
    pub questions: Vec<Question>,
    pub queued_prompts: Vec<String>,
    pub running: bool,
    pub status: String,
    pub usage: Usage,
    pub changed_from: usize,
    next_id: u64,
    draft: Option<usize>,
    reasoning: Option<usize>,
}

impl ChatModel {
    pub fn from_history(history: &[ChatMessage]) -> Self {
        let mut model = Self::default();
        for message in history {
            let start = model.entries.len();
            match &message.notice {
                Some(NoticeKind::Hidden | NoticeKind::CompactedMemory | NoticeKind::Host) => {
                    continue;
                }
                Some(NoticeKind::System | NoticeKind::Compacted | NoticeKind::Card(_)) => {
                    model.push(
                        EntryKind::Notice,
                        "",
                        message.content.as_deref().unwrap_or(""),
                    );
                    continue;
                }
                Some(NoticeKind::Error) => {
                    model.push(
                        EntryKind::Error,
                        "",
                        message.content.as_deref().unwrap_or(""),
                    );
                    continue;
                }
                _ => {}
            }
            let content = message.content.as_deref().unwrap_or("");
            match message.role {
                Role::System => {}
                Role::User => {
                    model.push(EntryKind::User, "", content);
                }
                Role::Assistant => {
                    if let Some(reasoning) =
                        message.reasoning.as_deref().filter(|text| !text.is_empty())
                    {
                        model.push(EntryKind::Reasoning, "", reasoning);
                    }
                    if !content.is_empty() {
                        model.push(EntryKind::Assistant, "", content);
                    }
                    for call in message.tool_calls.iter().flatten() {
                        let index = model.push(EntryKind::Tool, &call.name, "");
                        model.entries[index].detail = call.arguments.to_string();
                        model.entries[index].tool_call_id = Some(call.id.clone());
                        model.entries[index].complete = false;
                    }
                }
                Role::Tool => {
                    let previous = message.tool_call_id.as_deref().and_then(|id| {
                        model
                            .entries
                            .iter()
                            .rposition(|entry| entry.tool_call_id.as_deref() == Some(id))
                    });
                    let index = previous.unwrap_or_else(|| model.push(EntryKind::Tool, "结果", ""));
                    model.entries[index].text = content.to_owned();
                    model.entries[index].complete = true;
                    model.entries[index].ok = Some(!message.is_error.unwrap_or(false));
                    model.entries[index].timestamp = message.ts;
                    model.entries[index].duration_ms = message.duration_ms;
                }
            }
            for entry in &mut model.entries[start..] {
                entry.timestamp = message.ts;
                if entry.kind == EntryKind::Reasoning {
                    entry.duration_ms = message.duration_ms;
                }
            }
        }
        model
    }

    pub fn apply_event(&mut self, event: &UiEvent) {
        let previous_count = self.entries.len();
        self.changed_from = self.entries.len();
        match event {
            UiEvent::UserPrompt(text) => {
                self.finish_drafts();
                self.push(EntryKind::User, "", text);
                self.running = true;
            }
            UiEvent::UserSteer(text) => {
                self.push(EntryKind::User, "补充", text);
            }
            UiEvent::TextDelta {
                text,
                parent,
                child_id,
            } => {
                if parent.is_some() || child_id.is_some() {
                    self.child_delta(text, parent, *child_id);
                } else {
                    let index = self.draft.unwrap_or_else(|| {
                        let index = self.push(EntryKind::Assistant, "", "");
                        self.entries[index].complete = false;
                        self.draft = Some(index);
                        index
                    });
                    self.entries[index].text.push_str(text);
                    self.changed_from = self.changed_from.min(index);
                }
            }
            UiEvent::ReasoningDelta {
                text,
                parent,
                child_id,
            } => {
                if parent.is_some() || child_id.is_some() {
                    self.child_delta(text, parent, *child_id);
                } else {
                    let index = self.reasoning.unwrap_or_else(|| {
                        let index = self.push(EntryKind::Reasoning, "", "");
                        self.entries[index].complete = false;
                        self.reasoning = Some(index);
                        index
                    });
                    self.entries[index].text.push_str(text);
                    self.changed_from = self.changed_from.min(index);
                }
            }
            UiEvent::AssistantMessage(text) => {
                if let Some(index) = self.draft.take() {
                    self.entries[index].text = text.clone();
                    self.entries[index].complete = true;
                    self.entries[index].timestamp = Some(agent_llm::types::now_ms());
                    self.changed_from = self.changed_from.min(index);
                } else if !text.is_empty() {
                    self.push(EntryKind::Assistant, "", text);
                }
                if let Some(index) = self.reasoning.take() {
                    self.entries[index].complete = true;
                }
            }
            UiEvent::ToolCallStarted {
                name,
                args_preview,
                parent,
                child_id,
            } => {
                let index = self.push(EntryKind::Tool, name, "");
                let entry = &mut self.entries[index];
                entry.detail = args_preview.clone();
                entry.complete = false;
                entry.parent = parent.clone();
                entry.child_id = *child_id;
            }
            UiEvent::ToolCallFinished {
                name,
                ok,
                content,
                parent,
                child_id,
                ..
            } => {
                let previous = self.entries.iter().rposition(|entry| {
                    entry.kind == EntryKind::Tool
                        && !entry.complete
                        && entry.title == *name
                        && entry.child_id == *child_id
                        && entry.parent == *parent
                });
                let index = previous.unwrap_or_else(|| self.push(EntryKind::Tool, name, ""));
                self.entries[index].text = content.clone();
                self.entries[index].ok = Some(*ok);
                self.entries[index].complete = true;
                self.changed_from = self.changed_from.min(index);
                self.approvals.retain(|approval| {
                    approval.tool_name != *name || approval.child_id != *child_id
                });
            }
            UiEvent::ApprovalRequested {
                request_id,
                tool_name,
                diff,
                fuzzy,
                actions,
                child_id,
            } => {
                self.approvals.retain(|approval| {
                    approval.request_id != *request_id || approval.child_id != *child_id
                });
                self.approvals.push(Approval {
                    request_id: *request_id,
                    tool_name: tool_name.clone(),
                    diff: diff.clone(),
                    fuzzy: *fuzzy,
                    actions: actions.clone(),
                    child_id: *child_id,
                });
            }
            UiEvent::ApprovalTimedOut {
                request_id,
                child_id,
            } => {
                self.approvals.retain(|approval| {
                    approval.request_id != *request_id || approval.child_id != *child_id
                });
            }
            UiEvent::QuestionAsked {
                request_id,
                question,
                options,
            } => {
                self.questions
                    .retain(|entry| entry.request_id != *request_id);
                self.questions.push(Question {
                    request_id: *request_id,
                    text: question.clone(),
                    options: options.clone(),
                });
            }
            UiEvent::QuestionAnswered { request_id } | UiEvent::QuestionTimedOut { request_id } => {
                self.questions
                    .retain(|question| question.request_id != *request_id);
            }
            UiEvent::StateChanged(state) => {
                self.running = state.is_active();
                self.status = state.label();
                if !matches!(
                    state,
                    agent_ipc::AgentState::AwaitingToolConfirmation { .. }
                ) {
                    self.approvals
                        .retain(|approval| state.is_active() && approval.child_id.is_some());
                }
                if !self.running {
                    self.mark_drafts_complete();
                    self.approvals.clear();
                    self.questions.clear();
                }
            }
            UiEvent::SystemMessage(text) => {
                self.push(EntryKind::Notice, "", text);
            }
            UiEvent::Error(text) => {
                self.push(EntryKind::Error, "", text);
                self.mark_drafts_complete();
                self.running = false;
                self.approvals.clear();
                self.questions.clear();
            }
            UiEvent::Usage {
                prompt_tokens,
                completion_tokens,
                context_window,
                cached_tokens,
            } => {
                self.usage = Usage {
                    prompt_tokens: *prompt_tokens,
                    completion_tokens: *completion_tokens,
                    context_window: *context_window,
                    cached_tokens: *cached_tokens,
                };
            }
            UiEvent::Compacted {
                before_tokens,
                after_tokens,
                removed_messages,
                note,
                ..
            } => {
                self.push(
                    EntryKind::Notice,
                    "上下文整理",
                    &format!("{before_tokens} → {after_tokens} tokens · 合并 {removed_messages} 条消息\n{note}"),
                );
                self.usage.prompt_tokens = *after_tokens;
            }
            UiEvent::TurnRetry => {
                if let Some(index) = self
                    .entries
                    .iter()
                    .rposition(|entry| entry.kind == EntryKind::User)
                {
                    self.entries.truncate(index);
                    self.changed_from = index;
                }
                self.draft = None;
                self.reasoning = None;
                self.approvals.clear();
                self.questions.clear();
            }
            UiEvent::QueuedPrompts { items } => self.queued_prompts = items.clone(),
            UiEvent::Custom { kind, payload } if kind == "plan" => {
                self.push(EntryKind::Notice, "计划", &payload.to_string());
            }
            UiEvent::CompactionStarted { .. }
            | UiEvent::SessionToolWhitelisted { .. }
            | UiEvent::Custom { .. } => {}
        }
        for entry in self.entries.iter_mut().skip(previous_count) {
            entry.timestamp = Some(agent_llm::types::now_ms());
        }
    }

    fn push(&mut self, kind: EntryKind, title: &str, text: &str) -> usize {
        let index = self.entries.len();
        self.changed_from = self.changed_from.min(index);
        self.next_id += 1;
        self.entries.push(Entry {
            id: self.next_id,
            kind,
            title: title.to_owned(),
            text: text.to_owned(),
            detail: String::new(),
            complete: true,
            ok: None,
            timestamp: None,
            duration_ms: None,
            child_id: None,
            parent: None,
            tool_call_id: None,
        });
        index
    }

    fn child_delta(&mut self, text: &str, parent: &Option<String>, child_id: Option<u64>) {
        let index = self
            .entries
            .iter()
            .rposition(|entry| {
                entry.kind == EntryKind::Child
                    && entry.child_id == child_id
                    && entry.parent == *parent
            })
            .unwrap_or_else(|| {
                let index = self.push(EntryKind::Child, parent.as_deref().unwrap_or("子任务"), "");
                self.entries[index].parent = parent.clone();
                self.entries[index].child_id = child_id;
                index
            });
        self.entries[index].text.push_str(text);
        self.changed_from = self.changed_from.min(index);
    }

    fn finish_drafts(&mut self) {
        self.mark_drafts_complete();
        self.draft = None;
        self.reasoning = None;
    }

    fn mark_drafts_complete(&mut self) {
        for index in [self.draft, self.reasoning].into_iter().flatten() {
            self.entries[index].complete = true;
            self.changed_from = self.changed_from.min(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_ipc::UiEvent;
    use agent_llm::types::ChatMessage;

    #[test]
    fn final_message_replaces_its_stream_without_a_second_bubble() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::UserPrompt("你好".into()));
        model.apply_event(&UiEvent::TextDelta {
            text: "答".into(),
            child_id: None,
            parent: None,
        });
        model.apply_event(&UiEvent::TextDelta {
            text: "案".into(),
            child_id: None,
            parent: None,
        });
        model.apply_event(&UiEvent::AssistantMessage("答案".into()));
        let messages: Vec<_> = model
            .entries
            .iter()
            .filter(|entry| matches!(entry.kind, EntryKind::Assistant))
            .collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "答案");
        assert!(messages[0].complete);
    }

    #[test]
    fn terminal_state_before_final_text_does_not_duplicate_the_reply() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::UserPrompt("question".into()));
        model.apply_event(&UiEvent::TextDelta {
            text: "answer".into(),
            child_id: None,
            parent: None,
        });
        model.apply_event(&UiEvent::StateChanged(agent_ipc::AgentState::Finished));
        model.apply_event(&UiEvent::AssistantMessage("answer".into()));
        let replies: Vec<_> = model
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Assistant)
            .collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].text, "answer");
    }

    #[test]
    fn a_child_stream_does_not_become_the_parent_reply() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::TextDelta {
            text: "child".into(),
            child_id: Some(7),
            parent: Some("subagent".into()),
        });
        model.apply_event(&UiEvent::TextDelta {
            text: "parent".into(),
            child_id: None,
            parent: None,
        });
        let parent: Vec<_> = model
            .entries
            .iter()
            .filter(|entry| matches!(entry.kind, EntryKind::Assistant))
            .collect();
        assert_eq!(parent.len(), 1);
        assert_eq!(parent[0].text, "parent");
    }

    #[test]
    fn a_late_timeout_does_not_remove_a_new_approval() {
        let mut model = ChatModel::default();
        for request_id in [11, 12] {
            model.apply_event(&UiEvent::ApprovalRequested {
                request_id,
                tool_name: "bash".into(),
                diff: "echo ok".into(),
                fuzzy: false,
                actions: Vec::new(),
                child_id: None,
            });
        }
        model.apply_event(&UiEvent::ApprovalTimedOut {
            request_id: 11,
            child_id: None,
        });
        assert_eq!(model.approvals.len(), 1);
        assert_eq!(model.approvals[0].request_id, 12);
    }

    #[test]
    fn restored_history_and_completed_stream_show_the_same_text() {
        let history = vec![
            ChatMessage::user("question"),
            ChatMessage::assistant("answer"),
        ];
        let restored = ChatModel::from_history(&history);
        let mut streamed = ChatModel::default();
        streamed.apply_event(&UiEvent::UserPrompt("question".into()));
        streamed.apply_event(&UiEvent::TextDelta {
            text: "answer".into(),
            child_id: None,
            parent: None,
        });
        streamed.apply_event(&UiEvent::AssistantMessage("answer".into()));
        let text = |model: &ChatModel| {
            model
                .entries
                .iter()
                .map(|entry| (entry.kind.label().to_owned(), entry.text.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(text(&restored), text(&streamed));
    }

    #[test]
    fn retry_keeps_the_prompt_and_discards_the_failed_attempt() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::UserPrompt("question".into()));
        model.apply_event(&UiEvent::TextDelta {
            text: "partial".into(),
            child_id: None,
            parent: None,
        });
        model.apply_event(&UiEvent::TurnRetry);
        model.apply_event(&UiEvent::UserPrompt("question".into()));
        model.apply_event(&UiEvent::AssistantMessage("answer".into()));
        assert!(model.entries.iter().any(|entry| entry.text == "question"));
        assert!(!model.entries.iter().any(|entry| entry.text == "partial"));
        assert_eq!(
            model
                .entries
                .iter()
                .filter(|entry| matches!(entry.kind, EntryKind::Assistant))
                .count(),
            1
        );
    }
}
