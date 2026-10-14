//! The kernel's view of the session repository.
//!
//! The repository itself lives in `agent-store` — it names no product and no
//! kernel type, so it stands alone. Re-exported here so every existing
//! `session_store::X` path keeps working. The user preferences that share its
//! database live in the host (`husk_host::prefs`), not here.

pub use agent_store::*;
