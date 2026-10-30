use std::collections::HashMap;

use super::attachments::Attachment;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Draft {
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl Draft {
    /// Keep both a rejected draft and newer edits. Reselecting the same path
    /// replaces the older attachment, so its newer contents are not lost.
    fn merge(mut self, current: Draft) -> Draft {
        if self.text.is_empty() {
            self.text = current.text;
        } else if !current.text.is_empty() {
            self.text.push_str("\n\n");
            self.text.push_str(&current.text);
        }
        merge_attachments(&mut self.attachments, current.attachments);
        self
    }
}

pub(super) fn merge_attachments(attachments: &mut Vec<Attachment>, newer: Vec<Attachment>) {
    for attachment in newer {
        if let Some(existing) = attachments
            .iter_mut()
            .find(|existing| existing.source_path == attachment.source_path)
        {
            *existing = attachment;
        } else {
            attachments.push(attachment);
        }
    }
}

#[derive(Default)]
pub(super) struct Submissions {
    pending: HashMap<u64, Draft>,
}

impl Submissions {
    pub fn insert(&mut self, request: u64, draft: Draft) {
        self.pending.insert(request, draft);
    }

    pub fn acknowledge(&mut self, request: u64) {
        self.pending.remove(&request);
    }

    pub fn restore(&mut self, request: u64, current: Draft) -> Option<Draft> {
        self.pending
            .remove(&request)
            .map(|draft| draft.merge(current))
    }
}

#[cfg(test)]
mod tests {
    use super::{Draft, Submissions};
    use crate::features::chat::attachments::Attachment;

    fn text(value: &str) -> Draft {
        Draft {
            text: value.into(),
            ..Draft::default()
        }
    }

    #[test]
    fn failed_submission_preserves_both_original_and_new_input() {
        let mut submissions = Submissions::default();
        submissions.insert(1, text("first prompt"));
        assert_eq!(
            submissions.restore(1, text("draft in progress")),
            Some(text("first prompt\n\ndraft in progress"))
        );
        assert!(submissions.restore(1, text("")).is_none());
    }

    #[test]
    fn confirmed_submissions_are_not_restored() {
        let mut submissions = Submissions::default();
        submissions.insert(1, text("prompt"));
        submissions.insert(2, text("answer"));
        submissions.insert(3, text("another answer"));
        submissions.acknowledge(1);
        submissions.acknowledge(2);
        assert!(submissions.restore(1, text("")).is_none());
        assert!(submissions.restore(2, text("")).is_none());
        assert_eq!(
            submissions.restore(3, text("")),
            Some(text("another answer"))
        );
    }

    #[test]
    fn attachment_only_failure_keeps_new_input_and_newer_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.txt");
        let second_path = dir.path().join("second.txt");
        std::fs::write(&first_path, "before").unwrap();
        let original = Attachment::load(first_path.clone(), dir.path()).unwrap();
        std::fs::write(&first_path, "after").unwrap();
        std::fs::write(&second_path, "another").unwrap();
        let newer = Attachment::load(first_path, dir.path()).unwrap();
        let second = Attachment::load(second_path, dir.path()).unwrap();

        let mut submissions = Submissions::default();
        submissions.insert(
            7,
            Draft {
                text: String::new(),
                attachments: vec![original],
            },
        );
        let restored = submissions
            .restore(
                7,
                Draft {
                    text: "new text".into(),
                    attachments: vec![newer.clone(), second.clone()],
                },
            )
            .unwrap();
        assert_eq!(restored.text, "new text");
        assert_eq!(restored.attachments, vec![newer, second]);
    }
}
