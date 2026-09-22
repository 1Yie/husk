//! Skills — reusable instruction sets the agent loads on demand.
//!
//! The surface is deliberately split from the tools that reach it:
//!
//! ```text
//! scanner  →  which skills exist (frontmatter only, ~KB)
//! loader   →  one skill's body, read fresh at load time
//! prompt   →  the catalog block and the single instruction frame
//! manager  →  the entry point: cached catalog, fresh loads, suggestions
//! ```
//!
//! A skill is **knowledge, not capability**: loading one injects instructions
//! into the turn and nothing else. It cannot widen a tool's `readonly` flag,
//! change a `ToolClass`, or skip the permission gate — every action a skill
//! asks for still runs through the normal Tool → Capability → Policy → Audit
//! path, which is what makes third-party skill text safe to load.
//!
//! Consumers: the system prompt catalog (`session`), `$name`/`/{name}`
//! expansion and `/skills` (`commands`), the `skill` tool (`tools::skill`), and
//! the composer's `$` picker (app IPC, via [`scanner::scan_all_skills`]).

pub mod loader;
pub mod manager;
pub mod prompt;
pub mod scanner;

pub use loader::{LoadError, LoadedSkill};
pub use manager::SkillManager;
pub use scanner::SkillMetadata;
