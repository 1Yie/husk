//! agent-context — perception/cognition layer.
//!
//! Stage 1 scope: workspace file-tree scan + git snapshot.
//! Stage 6 added `hunks` (write tracking / undo).
//! Later stages add `budget` (truncation) and `memory` (LibSQL + FastEmbed)
//! per the capability roadmap.

pub mod git;
pub mod hunks;
pub mod memory;
pub mod workspace;

pub use git::{git_snapshot, GitSnapshot};
pub use hunks::{HunkTracker, TrackingMode, UndoOp};
pub use memory::{Episode, Fact, MemoryStore, Persona, TurnDistiller, TurnRecord};
pub use workspace::{WorkspaceScanner, WorkspaceTree};
