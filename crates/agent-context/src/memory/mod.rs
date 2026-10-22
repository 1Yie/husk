//! `memory` — hierarchical, evolving project cognition.
//!
//! Layers (capability-roadmap.md §1): working / session / **episodic** /
//! **semantic** / **persona**. `store` is the persistence backend (redb +
//! hash-embed, plus an optional sentence encoder for recall scoring);
//! `distill` is the post-turn background extractor.

pub mod distill;
pub mod semantic;
pub mod store;

pub use distill::{Summarizer, TurnDistiller, TurnRecord};
pub use semantic::init as init_semantic;
pub use store::{Episode, Fact, MemoryStore, Persona, PersonaScope, PurgeReport};
