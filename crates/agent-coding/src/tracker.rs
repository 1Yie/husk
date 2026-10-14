//! The coding product's write tracking — one shared [`HunkTracker`] behind the
//! file tools' [`WriteObserver`] seam.
//!
//! The kernel engine knows nothing about files. The file tools report every
//! change they apply through the observer installed on the tool context, and
//! this is what listens: the host reads it for undo/rewind and the files-changed
//! list, and [`crate::policy::CodingPolicy`] reads it for "what did this turn
//! touch" before the architecture check.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use agent_context::{HunkTracker, TrackingMode};
use agent_tools_fs::commit::{WriteObserver, WriteObserverSlot};

/// Cloning shares the tracker: the host keeps one handle, the context's
/// observer slot and the policy hold the others.
#[derive(Clone)]
pub struct WriteTracker(Arc<Mutex<HunkTracker>>);

impl WriteTracker {
    pub fn new(mode: TrackingMode) -> Self {
        Self(Arc::new(Mutex::new(HunkTracker::new(mode))))
    }

    /// Lock the tracker. A poisoned lock still yields the data — losing undo
    /// history to an unrelated panic would be worse than reading it.
    pub fn lock(&self) -> MutexGuard<'_, HunkTracker> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Files the current turn changed so far.
    pub fn touched_this_turn(&self) -> Vec<PathBuf> {
        let t = self.lock();
        t.files_in_turn(t.current_turn())
            .into_iter()
            .map(|p| p.to_path_buf())
            .collect()
    }

    /// Install this tracker on the context: as the file tools' write observer,
    /// and as itself so [`crate::policy::CodingPolicy::from_ctx`] can find it.
    pub fn install(&self, ctx: &mut agent_kernel::tools::registry::ToolCtx) {
        ctx.ext.insert(Arc::new(WriteObserverSlot(Arc::new(self.clone()))));
        ctx.ext.insert(Arc::new(self.clone()));
    }
}

impl WriteObserver for WriteTracker {
    fn on_write(&self, path: PathBuf, old: Option<Vec<u8>>, new: Vec<u8>) {
        self.lock().record_write(path, old, new, "agent");
    }
}
