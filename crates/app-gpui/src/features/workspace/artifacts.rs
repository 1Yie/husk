use std::collections::VecDeque;
use std::path::Path;

use super::browser::{self, EntryKind};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct OfficeArtifact {
    pub path: String,
    pub name: String,
    pub modified: u64,
}

/// Office's deliverable tree follows the same depth and newest-100 contract as
/// Tauri. Reuse pinned-directory/no-follow enumeration; call only on the owner.
pub fn recent_artifacts(root: &Path) -> Result<(Vec<OfficeArtifact>, bool), String> {
    let mut pending = VecDeque::from([(String::new(), 3usize)]);
    let mut files = Vec::new();
    let mut truncated = false;
    let mut directories = 0;
    while let Some((relative, depth)) = pending.pop_front() {
        if depth == 0 {
            continue;
        }
        if directories == 100 || files.len() >= 10_000 {
            truncated = true;
            break;
        }
        directories += 1;
        let listing = browser::list_directory(root, &relative)?;
        truncated |= listing.truncated;
        for entry in listing.entries {
            if entry.name.starts_with('.')
                || entry.name.starts_with("officecli") && entry.name.ends_with(".log")
            {
                continue;
            }
            match entry.kind {
                EntryKind::Directory => pending.push_back((entry.path, depth - 1)),
                EntryKind::File => files.push(OfficeArtifact {
                    path: entry.path,
                    name: entry.name,
                    modified: entry.modified,
                }),
                EntryKind::Symlink => {}
            }
        }
    }
    files.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    files.truncate(100);
    Ok((files, truncated))
}
