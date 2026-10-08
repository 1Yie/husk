//! agent-context — perception/cognition layer: workspace file-tree scan,
//! git snapshot, write tracking (`hunks`), hierarchical `memory`, and the
//! `.blueprint/` architecture layer.

pub mod blueprint;
pub mod git;
pub mod hunks;
pub mod memory;
pub mod workspace;

pub use blueprint::Blueprint;
pub use git::{git_snapshot, GitSnapshot};
pub use hunks::{HunkTracker, TrackingMode, UndoError, UndoOp};
pub use memory::{Episode, Fact, MemoryStore, Persona, TurnDistiller, TurnRecord};
pub use workspace::{WorkspaceScanner, WorkspaceTree};
