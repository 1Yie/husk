//! What a context has already observed — enough to say "this is the same
//! output you got before" or "this file changed since you read it", never
//! enough to answer in the tool's place.
//!
//! The ledger stores digests, not content: a repeat is always re-executed and
//! returned in full, so a stale cache can never stand in for the workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use agent_kernel::tools::registry::ToolCtx;
use xxhash_rust::xxh3::xxh3_64;

/// Entries kept before the ledger starts over. A long session forgets its
/// oldest observations rather than growing without bound; forgetting only
/// costs a note, never a result.
const MAX_ENTRIES: usize = 2048;

enum Outcome {
    Ok(u64),
    Err(String),
}

#[derive(Default)]
struct Inner {
    /// Canonical call key → what that call last returned.
    calls: HashMap<String, Outcome>,
    /// Resolved file → the `content_hash` its last read reported.
    files: HashMap<PathBuf, String>,
}

/// One per context. A batch's derived context clones the extension table and
/// so shares it; a delegated child gets its own, because the child cannot see
/// what the parent read.
#[derive(Default)]
pub struct ObservationLedger(Mutex<Inner>);

impl ObservationLedger {
    /// Install a fresh ledger on `ctx` and hand back the shared handle.
    pub fn install(ctx: &mut ToolCtx) -> Arc<Self> {
        let ledger = Arc::new(Self::default());
        ctx.ext.insert(ledger.clone());
        ledger
    }

    /// Forget everything — the history that held the earlier results is gone.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.calls.clear();
        inner.files.clear();
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record a successful output. `true` when the identical call already
    /// returned this identical output.
    pub(crate) fn record_ok(&self, key: &str, output: &str) -> bool {
        let digest = xxh3_64(output.as_bytes());
        let mut inner = self.lock();
        if inner.calls.len() >= MAX_ENTRIES && !inner.calls.contains_key(key) {
            inner.calls.clear();
        }
        let repeat = matches!(inner.calls.get(key), Some(Outcome::Ok(d)) if *d == digest);
        inner.calls.insert(key.to_string(), Outcome::Ok(digest));
        repeat
    }

    /// Record a failure. `true` when the identical call already failed with
    /// the identical error.
    pub(crate) fn record_err(&self, key: &str, error: &str) -> bool {
        let mut inner = self.lock();
        if inner.calls.len() >= MAX_ENTRIES && !inner.calls.contains_key(key) {
            inner.calls.clear();
        }
        let repeat = matches!(inner.calls.get(key), Some(Outcome::Err(e)) if e == error);
        inner
            .calls
            .insert(key.to_string(), Outcome::Err(error.to_string()));
        repeat
    }

    /// Record the hash a read reported for `path`. Returns the previous hash
    /// when it differs — the file changed between two reads.
    pub(crate) fn record_file(&self, path: PathBuf, hash: &str) -> Option<String> {
        let mut inner = self.lock();
        if inner.files.len() >= MAX_ENTRIES && !inner.files.contains_key(&path) {
            inner.files.clear();
        }
        match inner.files.insert(path, hash.to_string()) {
            Some(old) if old != hash => Some(old),
            _ => None,
        }
    }

    /// The hash the last read of `path` reported.
    pub(crate) fn file_hash(&self, path: &Path) -> Option<String> {
        self.lock().files.get(path).cloned()
    }
}

/// The hash `smart_read` reports for a file's content — same function, same
/// format, so a shell read can be compared against it.
pub(crate) fn content_hash(content: &str) -> String {
    format!("{:016x}", xxh3_64(content.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeat_needs_the_same_call_and_the_same_output() {
        let l = ObservationLedger::default();
        assert!(!l.record_ok("read a", "v1"));
        assert!(l.record_ok("read a", "v1"));
        assert!(
            !l.record_ok("read a", "v2"),
            "changed output is not a repeat"
        );
        assert!(!l.record_ok("read b", "v2"), "another call is not a repeat");
    }

    #[test]
    fn a_failure_repeats_only_with_the_same_error() {
        let l = ObservationLedger::default();
        assert!(!l.record_err("k", "missing"));
        assert!(l.record_err("k", "missing"));
        assert!(!l.record_err("k", "denied"));
        // A success in between resets the streak.
        l.record_ok("k", "out");
        assert!(!l.record_err("k", "denied"));
    }

    #[test]
    fn a_changed_file_reports_its_previous_hash() {
        let l = ObservationLedger::default();
        let p = PathBuf::from("/w/a.rs");
        assert_eq!(l.record_file(p.clone(), "h1"), None);
        assert_eq!(l.record_file(p.clone(), "h1"), None);
        assert_eq!(l.record_file(p.clone(), "h2").as_deref(), Some("h1"));
        assert_eq!(l.file_hash(&p).as_deref(), Some("h2"));
        l.clear();
        assert_eq!(l.file_hash(&p), None);
    }
}
