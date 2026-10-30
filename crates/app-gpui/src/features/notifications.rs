use std::collections::{HashMap, HashSet};

use agent_ipc::{AgentState, UiEvent};

use crate::bridge::SessionKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    Approval,
    Question,
    Finished,
    Failed,
}

impl Notice {
    /// Fixed copy only: session titles, prompts and tool output stay private.
    pub fn body(self) -> &'static str {
        match self {
            Self::Approval => "有会话等待确认。",
            Self::Question => "有会话等待回答。",
            Self::Finished => "有会话已完成。",
            Self::Failed => "有会话运行失败。",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoppedState {
    Approval,
    Finished,
    Failed,
    Cancelled,
}

#[derive(Default)]
struct SessionNotices {
    last_state: Option<StoppedState>,
    questions: HashSet<u64>,
    child_approvals: HashSet<(u64, u64)>,
}

#[derive(Default)]
pub struct NotificationPolicy {
    sessions: HashMap<SessionKey, SessionNotices>,
}

impl NotificationPolicy {
    /// A visible session already presents the event; it must not produce an
    /// OS notification, even if the same event is repeated after focus changes.
    pub fn observe(&mut self, key: &SessionKey, event: &UiEvent, visible: bool) -> Option<Notice> {
        if !matches!(
            event,
            UiEvent::StateChanged(_)
                | UiEvent::UserPrompt(_)
                | UiEvent::QuestionAsked { .. }
                | UiEvent::QuestionAnswered { .. }
                | UiEvent::QuestionTimedOut { .. }
                | UiEvent::ApprovalRequested {
                    child_id: Some(_),
                    ..
                }
                | UiEvent::ApprovalTimedOut {
                    child_id: Some(_),
                    ..
                }
                | UiEvent::ToolCallFinished {
                    child_id: Some(_),
                    ..
                }
        ) {
            return None;
        }
        let session = self.sessions.entry(key.clone()).or_default();
        let notice = match event {
            UiEvent::UserPrompt(_) => {
                session.last_state = None;
                session.questions.clear();
                session.child_approvals.clear();
                None
            }
            UiEvent::StateChanged(state) => {
                let (stopped, notice) = match state {
                    AgentState::AwaitingToolConfirmation { .. }
                    | AgentState::AwaitingPluginConsent { .. }
                    | AgentState::AwaitingConsent { .. } => {
                        (StoppedState::Approval, Some(Notice::Approval))
                    }
                    AgentState::Finished => (StoppedState::Finished, Some(Notice::Finished)),
                    AgentState::Failed(reason) if reason == "turn cancelled by user" => {
                        (StoppedState::Cancelled, None)
                    }
                    AgentState::Failed(_) => (StoppedState::Failed, Some(Notice::Failed)),
                    _ => {
                        session.last_state = None;
                        if !state.is_active() {
                            session.questions.clear();
                            session.child_approvals.clear();
                        }
                        return None;
                    }
                };
                if !state.is_active() {
                    session.questions.clear();
                    session.child_approvals.clear();
                }
                let duplicate = session.last_state == Some(stopped);
                session.last_state = Some(stopped);
                if duplicate { None } else { notice }
            }
            UiEvent::QuestionAsked { request_id, .. } => session
                .questions
                .insert(*request_id)
                .then_some(Notice::Question),
            UiEvent::QuestionAnswered { request_id } | UiEvent::QuestionTimedOut { request_id } => {
                session.questions.remove(request_id);
                None
            }
            UiEvent::ApprovalRequested {
                request_id,
                child_id: Some(child),
                ..
            } => session
                .child_approvals
                .insert((*child, *request_id))
                .then_some(Notice::Approval),
            UiEvent::ApprovalTimedOut {
                request_id,
                child_id: Some(child),
            } => {
                session.child_approvals.remove(&(*child, *request_id));
                None
            }
            UiEvent::ToolCallFinished {
                child_id: Some(child),
                ..
            } => {
                session.child_approvals.retain(|(id, _)| id != child);
                None
            }
            _ => None,
        };
        if visible { None } else { notice }
    }

    pub fn forget(&mut self, key: &SessionKey) {
        self.sessions.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(root: &str, id: i64) -> SessionKey {
        SessionKey {
            root: root.into(),
            id,
        }
    }

    #[test]
    fn foreground_and_duplicate_terminal_states_are_suppressed() {
        let mut policy = NotificationPolicy::default();
        let key = key("/a", 1);
        let finished = UiEvent::StateChanged(AgentState::Finished);
        assert_eq!(policy.observe(&key, &finished, true), None);
        assert_eq!(policy.observe(&key, &finished, false), None);
        policy.observe(&key, &UiEvent::UserPrompt("next".into()), false);
        assert_eq!(
            policy.observe(&key, &finished, false),
            Some(Notice::Finished)
        );
        assert_eq!(policy.observe(&key, &finished, false), None);
        policy.forget(&key);
        assert_eq!(
            policy.observe(&key, &finished, false),
            Some(Notice::Finished)
        );
    }

    #[test]
    fn cancellation_is_silent_and_sessions_are_isolated_by_root() {
        let mut policy = NotificationPolicy::default();
        let a = key("/a", 1);
        let b = key("/b", 1);
        assert_eq!(
            policy.observe(
                &a,
                &UiEvent::StateChanged(AgentState::Failed("turn cancelled by user".into())),
                false,
            ),
            None
        );
        let failure = UiEvent::StateChanged(AgentState::Failed("private detail".into()));
        assert_eq!(policy.observe(&a, &failure, false), Some(Notice::Failed));
        assert_eq!(policy.observe(&b, &failure, false), Some(Notice::Failed));
        assert!(!Notice::Failed.body().contains("private detail"));
    }

    #[test]
    fn questions_and_child_approvals_are_deduplicated_by_request() {
        let mut policy = NotificationPolicy::default();
        let key = key("/a", 7);
        let question = UiEvent::QuestionAsked {
            request_id: 4,
            question: "private question".into(),
            options: Vec::new(),
        };
        assert_eq!(
            policy.observe(&key, &question, false),
            Some(Notice::Question)
        );
        assert_eq!(policy.observe(&key, &question, false), None);
        policy.observe(&key, &UiEvent::QuestionAnswered { request_id: 4 }, false);
        assert_eq!(
            policy.observe(&key, &question, false),
            Some(Notice::Question)
        );
        policy.observe(&key, &UiEvent::StateChanged(AgentState::Idle), false);
        assert_eq!(
            policy.observe(&key, &question, false),
            Some(Notice::Question)
        );
        let child = UiEvent::ApprovalRequested {
            request_id: 8,
            tool_name: "private tool".into(),
            diff: "private diff".into(),
            fuzzy: false,
            actions: Vec::new(),
            child_id: Some(9),
        };
        assert_eq!(policy.observe(&key, &child, false), Some(Notice::Approval));
        assert_eq!(policy.observe(&key, &child, false), None);
        assert!(!Notice::Approval.body().contains("private"));
    }
}
