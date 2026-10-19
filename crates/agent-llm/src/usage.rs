//! Host-visible cache accounting.
//!
//! The normalized `StreamChunk::Done` carries prompt/completion/cached tokens
//! but predates provider cache *writes* (Anthropic's
//! `cache_creation_input_tokens`). Changing that shared wire type would touch
//! every kernel/host consumer, so the provider publishes the extra counter
//! here and the session host attaches one observer per session.
//!
//! The value is the **last streamed round's** cache-write count. A turn can
//! contain several provider rounds; the host reads it when forwarding that
//! round's `Usage` event and again at turn end for persistence.

use std::sync::atomic::{AtomicU32, Ordering};

/// Per-session cache-write counter. Cheap to clone (one `Arc`), lock-free.
#[derive(Debug, Default)]
pub struct UsageObserver {
    cache_write: AtomicU32,
}

impl UsageObserver {
    /// Clear before a request so a provider that reports no cache write cannot
    /// inherit the previous round's (or previous provider's) value.
    pub fn reset(&self) {
        self.cache_write.store(0, Ordering::Relaxed);
    }

    /// Record the current round's cache-write tokens.
    pub fn set_cache_write(&self, tokens: u32) {
        self.cache_write.store(tokens, Ordering::Relaxed);
    }

    /// The current round's cache-write tokens (0 when unsupported).
    pub fn cache_write(&self) -> u32 {
        self.cache_write.load(Ordering::Relaxed)
    }
}
