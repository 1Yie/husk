//! Presentation-only turn boundaries. Entry ids remain owned by ChatModel:
//! streaming deltas, retries and child routing never acquire UI identities.
use std::ops::Range;

use super::model::{Entry, EntryKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Turn {
    pub id: u64,
    pub entries: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Entry(usize),
    Tools(Range<usize>),
}

impl Turn {
    pub fn phases(&self, entries: &[Entry]) -> Vec<Phase> {
        let mut phases = Vec::new();
        let mut index = self.entries.start;
        while index < self.entries.end {
            if matches!(entries[index].kind, EntryKind::Tool | EntryKind::Child) {
                let start = index;
                while index < self.entries.end
                    && matches!(entries[index].kind, EntryKind::Tool | EntryKind::Child)
                {
                    index += 1;
                }
                phases.push(Phase::Tools(start..index));
            } else {
                phases.push(Phase::Entry(index));
                index += 1;
            }
        }
        phases
    }

    /// Copy the assistant's actual prose, not reasoning, tool output or notices.
    pub fn copy_text(&self, entries: &[Entry]) -> String {
        entries[self.entries.clone()]
            .iter()
            .filter(|entry| entry.kind == EntryKind::Assistant)
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn assistant_timestamp(&self, entries: &[Entry]) -> Option<i64> {
        entries[self.entries.clone()]
            .iter()
            .rev()
            .find(|entry| entry.kind == EntryKind::Assistant)
            .and_then(|entry| entry.timestamp)
    }

    pub fn elapsed(&self, entries: &[Entry]) -> Option<std::time::Duration> {
        let start = entries[self.entries.clone()]
            .iter()
            .find(|entry| entry.kind == EntryKind::User && entry.title != "补充")?
            .timestamp?;
        let end = self.assistant_timestamp(entries)?;
        (end > start).then(|| std::time::Duration::from_millis((end - start) as u64))
    }
}

pub(super) fn timestamp_label(timestamp: i64) -> Option<String> {
    use chrono::{Datelike, TimeZone};
    let time = chrono::Local.timestamp_millis_opt(timestamp).single()?;
    let now = chrono::Local::now();
    Some(
        time.format(if time.date_naive() == now.date_naive() {
            "%H:%M"
        } else if time.year() == now.year() {
            "%m-%d %H:%M"
        } else {
            "%Y-%m-%d %H:%M"
        })
        .to_string(),
    )
}

pub(super) fn project(entries: &[Entry]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        // A steer is a bubble inside the current run, not a new turn.
        let opens = entry.kind == EntryKind::User && entry.title != "补充";
        if opens || turns.is_empty() {
            turns.push(Turn {
                id: entry.id,
                entries: index..index + 1,
            });
        } else if let Some(turn) = turns.last_mut() {
            turn.entries.end = index + 1;
        }
    }
    turns
}

pub(super) fn changed_turn(turns: &[Turn], entry_index: usize) -> usize {
    turns
        .partition_point(|turn| turn.entries.end <= entry_index)
        .min(turns.len().saturating_sub(1))
}

/// Reproject only the changed suffix. A streaming token does not walk old
/// messages, clone their text, or rebuild every settled turn.
pub(super) fn update(turns: &mut Vec<Turn>, entries: &[Entry], changed_entry: usize) {
    let changed = changed_turn(turns, changed_entry);
    let start = turns.get(changed).map_or(0, |turn| turn.entries.start);
    turns.truncate(changed);
    for (index, entry) in entries.iter().enumerate().skip(start) {
        let opens = entry.kind == EntryKind::User && entry.title != "补充";
        if index == start || opens {
            turns.push(Turn {
                id: entry.id,
                entries: index..index + 1,
            });
        } else if let Some(turn) = turns.last_mut() {
            turn.entries.end = index + 1;
        }
    }
}

pub(super) fn elapsed_label(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::chat::model::ChatModel;
    use agent_ipc::UiEvent;

    fn delta(text: &str) -> UiEvent {
        UiEvent::TextDelta {
            text: text.into(),
            parent: None,
            child_id: None,
        }
    }

    #[test]
    fn stream_updates_keep_turn_and_answer_identity() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::UserPrompt("first".into()));
        model.apply_event(&delta("par"));
        let turn = project(&model.entries)[0].id;
        let answer = model.entries[1].id;
        model.apply_event(&delta("tial"));
        model.apply_event(&UiEvent::AssistantMessage("answer".into()));
        assert_eq!(project(&model.entries)[0].id, turn);
        assert_eq!(model.entries[1].id, answer);
        assert_eq!(
            project(&model.entries)[0].copy_text(&model.entries),
            "answer"
        );
    }

    #[test]
    fn steer_is_inside_turn_and_next_prompt_opens_another() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::UserPrompt("first".into()));
        model.apply_event(&UiEvent::UserSteer("also".into()));
        model.apply_event(&UiEvent::AssistantMessage("answer".into()));
        model.apply_event(&UiEvent::UserPrompt("second".into()));
        let turns = project(&model.entries);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].entries, 0..3);
        assert_eq!(turns[1].entries, 3..4);
        assert_eq!(changed_turn(&turns, 1), 0);
    }

    #[test]
    fn tools_group_without_reordering_reasoning_or_prose() {
        let mut model = ChatModel::default();
        model.apply_event(&UiEvent::UserPrompt("task".into()));
        model.apply_event(&UiEvent::ReasoningDelta {
            text: "private reasoning".into(),
            parent: None,
            child_id: None,
        });
        for name in ["read", "write"] {
            model.apply_event(&UiEvent::ToolCallStarted {
                name: name.into(),
                args_preview: "args".into(),
                parent: None,
                child_id: None,
            });
        }
        model.apply_event(&UiEvent::AssistantMessage("final".into()));
        let turn = &project(&model.entries)[0];
        assert_eq!(
            turn.phases(&model.entries),
            [
                Phase::Entry(0),
                Phase::Entry(1),
                Phase::Tools(2..4),
                Phase::Entry(4)
            ]
        );
        assert_eq!(turn.copy_text(&model.entries), "final");
    }

    #[test]
    fn retry_preserves_prior_turns_and_sessions_project_independently() {
        let mut first = ChatModel::default();
        first.apply_event(&UiEvent::UserPrompt("one".into()));
        first.apply_event(&UiEvent::AssistantMessage("done".into()));
        first.apply_event(&UiEvent::UserPrompt("two".into()));
        first.apply_event(&delta("partial"));
        let id = project(&first.entries)[0].id;
        first.apply_event(&UiEvent::TurnRetry);
        assert_eq!(project(&first.entries).len(), 1);
        assert_eq!(project(&first.entries)[0].id, id);
        assert!(project(&ChatModel::default().entries).is_empty());
    }

    #[test]
    fn incremental_projection_matches_rebuild_across_retry_and_stream() {
        let mut model = ChatModel::default();
        let mut turns = Vec::new();
        for event in [
            UiEvent::UserPrompt("one".into()),
            delta("first"),
            UiEvent::AssistantMessage("first".into()),
            UiEvent::UserPrompt("two".into()),
            delta("partial"),
            UiEvent::UserSteer("extra".into()),
            delta(" more"),
            UiEvent::TurnRetry,
            UiEvent::UserPrompt("retry".into()),
            delta("done"),
        ] {
            model.apply_event(&event);
            update(&mut turns, &model.entries, model.changed_from);
            assert_eq!(turns, project(&model.entries));
        }
    }

    #[test]
    fn replay_uses_persisted_timestamps_and_omits_missing_durations() {
        use agent_llm::types::ChatMessage;
        let mut user = ChatMessage::user("task");
        user.ts = Some(1_000);
        let mut answer = ChatMessage::assistant("answer");
        answer.ts = Some(3_500);
        let model = ChatModel::from_history(&[user.clone(), answer.clone()]);
        let turn = &project(&model.entries)[0];
        assert_eq!(
            turn.elapsed(&model.entries),
            Some(std::time::Duration::from_millis(2_500))
        );
        user.ts = None;
        answer.ts = None;
        let model = ChatModel::from_history(&[user, answer]);
        assert_eq!(project(&model.entries)[0].elapsed(&model.entries), None);
        assert!(model.entries.iter().all(|entry| entry.timestamp.is_none()));
    }
}
