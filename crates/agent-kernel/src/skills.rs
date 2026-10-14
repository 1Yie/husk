//! Skills — re-exported from the `agent-skills` crate.
//!
//! Skill scanning, loading and prompt rendering know nothing about the kernel,
//! so they live in their own crate. This module keeps the `crate::skills::...`
//! paths working, including the submodule paths (`crate::skills::scanner::…`),
//! which the glob re-export preserves.

pub use agent_skills::*;
