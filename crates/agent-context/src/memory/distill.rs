//! `memory/distill.rs` — the post-turn background extractor.
//!
//! Contract (capability-roadmap.md §Write path): at `Finished`/`Failed`,
//! spawn a low-priority task — summarize the turn → extract candidate facts
//! → dedupe → write episode + facts. **Never blocks the turn.**
//!
//! The distiller is deliberately model-agnostic: the engine hands it a
//! plain `TurnRecord` and a `summarize` callback (the kernel supplies the
//! active provider's text-generation), so this crate stays provider-free.

use std::sync::Arc;

use super::store::{Episode, MemoryStore};

/// What one finished turn left behind — the distiller's input.
pub struct TurnRecord {
    /// One-line summary of the user's ask.
    pub task: String,
    /// `"success" | "failed" | "steered"`.
    pub outcome: String,
    /// Files the turn wrote (from the HunkTracker).
    pub files: Vec<String>,
    /// A user correction/deny reason — highest-value fact seed.
    pub correction: Option<String>,
    /// Steering text injected mid-turn, if any.
    pub steered_with: Option<String>,
}

/// Extract candidate facts from a turn — deterministic rules (the LLM
/// summarize variant lands when a cheap model is configured):
///
/// - A **user correction** (`deny_tool` reason or steering) → a fact at
///   `confidence: 0.6`.
/// - A **repeated outcome** (same task family succeeding) → confidence bump
///   via the store's dedupe merge.
/// - Anything matching the sanitizer denylist is **never** distilled.
pub struct TurnDistiller {
    store: Arc<MemoryStore>,
}

impl TurnDistiller {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }

    /// Fire-and-forget distill — runs in a spawned task, never blocks.
    pub fn spawn_distill(
        self: &Arc<Self>,
        record: TurnRecord,
    ) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            this.distill(record).await;
        })
    }

    /// The synchronous distill body — extracted for tests.
    pub async fn distill(&self, record: TurnRecord) {
        // 1. Always write the episode — it's the raw "what happened".
        let episode = Episode {
            id: 0,
            workspace_id: self.store.workspace_id(),
            task: record.task.clone(),
            outcome: record.outcome.clone(),
            files: record.files.clone(),
            correction: record.correction.clone(),
            created_at: now(),
        };
        let _ = self.store.add_episode(episode);

        // 2. Corrections seed candidate facts at 0.6 confidence.
        if let Some(c) = &record.correction {
            if !looks_secret(c) {
                let _ = self.store.upsert_fact(
                    format!("user corrected: {c}"),
                    0.6,
                );
            }
        }
        if let Some(steer) = &record.steered_with {
            if !looks_secret(steer) {
                let _ = self.store.upsert_fact(
                    format!("user steered: {steer}"),
                    0.6,
                );
            }
        }
    }
}

/// Never distill secrets — the same denylist patterns as env sanitization.
fn looks_secret(text: &str) -> bool {
    let t = text.to_uppercase();
    ["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "PRIVATE", "AWS_", "GITHUB_"]
        .iter()
        .any(|p| t.contains(p))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
