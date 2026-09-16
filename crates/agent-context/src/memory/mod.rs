//! `memory` — hierarchical, evolving project cognition.
//!
//! Layers (capability-roadmap.md §1): working / session / **episodic** /
//! **semantic** / **persona**. `store` is the persistence backend (redb +
//! hash-embed; LibSQL+FastEmbed is the spec target behind the same API).
//! `distill` is the post-turn background extractor.

pub mod distill;
pub mod store;

pub use distill::{TurnDistiller, TurnRecord};
pub use store::{Episode, Fact, MemoryStore, Persona};
