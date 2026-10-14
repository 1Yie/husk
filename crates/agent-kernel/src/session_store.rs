//! The kernel's view of the session repository.
//!
//! The repository itself lives in `agent-store` — it names no product and no
//! kernel type, so it stands alone. The preferences it shares a database with
//! stay in `crate::prefs`. Both are re-exported here so every existing
//! `session_store::X` path keeps working.

pub use agent_store::*;

pub use crate::prefs::*;
