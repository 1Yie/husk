use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Add,
    Delete,
    Context,
    Hunk,
    Meta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: RowKind,
    pub text: String,
    pub old: Option<usize>,
    pub new: Option<usize>,
}

/// Parse a unified patch as literal text, never Markdown/HTML. Numbers come
/// from each hunk, including zero-length ranges and omitted ",1" counts.
pub fn parse_patch(patch: &str) -> Vec<DiffRow> {
    let mut old = None;
    let mut new = None;
    patch
        .lines()
        .filter_map(|line| {
            if line.starts_with("diff --git ") {
                old = None;
                new = None;
                return None;
            }
            if line.starts_with("\\")
                || line.starts_with("index ")
                || line.starts_with("new file mode ")
                || line.starts_with("deleted file mode ")
                || line.starts_with("old mode ")
                || line.starts_with("new mode ")
                || line.starts_with("similarity index ")
                || line.starts_with("rename from ")
                || line.starts_with("rename to ")
                || ((line.starts_with("--- ") || line.starts_with("+++ "))
                    && (old.is_none() || line.starts_with("--- a/") || line.starts_with("+++ b/")))
            {
                return None;
            }
            let mut row = DiffRow {
                kind: RowKind::Meta,
                text: line.into(),
                old: None,
                new: None,
            };
            if line.starts_with("@@ ") {
                let mut ranges = line.split_whitespace().skip(1);
                old = ranges.next().and_then(range_start);
                new = ranges.next().and_then(range_start);
                row.kind = RowKind::Hunk;
            } else if old.is_some() && new.is_some() {
                match line.as_bytes().first() {
                    Some(b'+') => {
                        row.kind = RowKind::Add;
                        row.new = new;
                        new = new.map(|n| n.saturating_add(1));
                        row.text = line[1..].into();
                    }
                    Some(b'-') => {
                        row.kind = RowKind::Delete;
                        row.old = old;
                        old = old.map(|n| n.saturating_add(1));
                        row.text = line[1..].into();
                    }
                    Some(b' ') => {
                        row.kind = RowKind::Context;
                        row.old = old;
                        row.new = new;
                        old = old.map(|n| n.saturating_add(1));
                        new = new.map(|n| n.saturating_add(1));
                        row.text = line[1..].into();
                    }
                    _ => {}
                }
            }
            Some(row)
        })
        .collect()
}

fn range_start(range: &str) -> Option<usize> {
    range
        .strip_prefix(['-', '+'])?
        .split(',')
        .next()?
        .parse()
        .ok()
}

/// Small lists start expanded. Crossing the budget is sticky: existing
/// expanded rows stay open, but newly arriving rows default to collapsed.
#[derive(Default)]
pub struct Expansion {
    dense: bool,
    previous: HashSet<String>,
    opened: HashSet<String>,
    closed: HashSet<String>,
}

impl Expansion {
    pub fn reconcile(&mut self, paths: &[String], patches: usize) {
        if !self.dense && (paths.len() > 4 || patches > 8) {
            self.dense = true;
            self.opened = self.previous.difference(&self.closed).cloned().collect();
            self.closed.clear();
        }
        let current: HashSet<_> = paths.iter().cloned().collect();
        self.opened.retain(|path| current.contains(path));
        self.closed.retain(|path| current.contains(path));
        self.previous = current;
    }

    pub fn is_open(&self, path: &str) -> bool {
        if self.dense {
            self.opened.contains(path)
        } else {
            !self.closed.contains(path)
        }
    }

    pub fn toggle(&mut self, path: &str) {
        let set = if self.dense {
            &mut self.opened
        } else {
            &mut self.closed
        };
        if !set.remove(path) {
            set.insert(path.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_hunks_track_independent_gutters_and_no_newline_metadata() {
        let rows = parse_patch(
            "--- a/x\n+++ b/x\n@@ -3,2 +7,2 @@ function\n same\n-old\n+new\n\\ No newline at end of file\n@@ -0,0 +1 @@\n+first\n",
        );
        assert_eq!((rows[1].old, rows[1].new), (Some(3), Some(7)));
        assert_eq!((rows[2].old, rows[2].new), (Some(4), None));
        assert_eq!((rows[3].old, rows[3].new), (None, Some(8)));
        assert_eq!(rows.len(), 6);
        assert_eq!((rows[5].old, rows[5].new), (None, Some(1)));
        assert_eq!(rows[5].text, "first");
    }

    #[test]
    fn dense_mode_is_sticky_preserves_reader_and_honors_manual_closure() {
        let mut state = Expansion::default();
        state.reconcile(&["a".into(), "b".into()], 2);
        assert!(state.is_open("a"));
        state.toggle("b");
        state.reconcile(
            &["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            5,
        );
        assert!(state.is_open("a"));
        assert!(!state.is_open("b"));
        assert!(!state.is_open("e"));
        state.toggle("e");
        assert!(state.is_open("e"));
        state.reconcile(&["c".into()], 1);
        assert!(!state.is_open("c"));
    }

    #[test]
    fn initial_large_list_does_not_mount_all_patches() {
        let mut state = Expansion::default();
        let paths = (0..10).map(|n| n.to_string()).collect::<Vec<_>>();
        state.reconcile(&paths, 10);
        assert!(paths.iter().all(|p| !state.is_open(p)));
    }
}
