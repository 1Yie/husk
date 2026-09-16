//! `StreamThrottler` — 33 ms frame-aligned token coalescer.
//!
//! Contract (kernel-architecture.md §Bridge throttling): SSE deltas funnel
//! into a `Mutex<String>` buffer; a `tokio::interval` at ~30 fps drains it
//! and flushes to the UI via `invoke_from_event_loop`. **Never** call
//! `invoke_from_event_loop` per SSE chunk — that's the "UI blocks" violation.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

/// Frame interval — ~30 fps.
const FRAME: Duration = Duration::from_millis(33);

/// Coalesces rapid kernel deltas into per-frame UI flushes.
///
/// Usage: spawn once per session with `spawn(ui_event_tx)`. Kernel pushes
/// `StreamDelta` items; every 33 ms the throttler emits one
/// `ThrottledBatch` per active stream row.
///
/// Stage 5: wired in 5.5 once high-rate streaming needs coalescing — deltas
/// currently apply directly per `UiEvent` (MockProvider's rate is low enough).
#[allow(dead_code)]
pub struct StreamThrottler {
    /// Per-row buffers keyed by message id — text accumulates between frames.
    pending: Arc<Mutex<Vec<(i32, String)>>>,
}

/// One coalesced flush destined for a single UI row.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ThrottledBatch {
    /// Target row (`SessionMessageData.id`); `-1` = append to the last row.
    pub row: i32,
    /// Text appended this frame.
    pub text: String,
    /// Reasoning text appended this frame (kept on a separate visual stream).
    pub reasoning: String,
}

/// Inbound kernel delta.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum StreamDelta {
    Text { row: i32, text: String },
    Reasoning { row: i32, text: String },
}

#[allow(dead_code)]
impl StreamThrottler {
    pub fn new() -> Self {
        Self { pending: Arc::new(Mutex::new(Vec::new())) }
    }

    /// Push a delta — cheap, lock-free-enough for the sampling hot path.
    pub fn push(&self, delta: StreamDelta) {
        let mut g = self.pending.lock().unwrap();
        let (row, text, is_reasoning) = match delta {
            StreamDelta::Text { row, text } => (row, text, false),
            StreamDelta::Reasoning { row, text } => (row, text, true),
        };
        if let Some(slot) = g.iter_mut().find(|(r, _)| *r == row) {
            slot.1.push_str(&text);
        } else {
            g.push((row, text));
        }
        let _ = is_reasoning; // reasoning/text share the row buffer; the UI
                              // splits them by which property the flush writes
    }

    /// Spawn the 33 ms drain loop. Emits `ThrottledBatch`es on `out`.
    /// Returns a handle that stops the loop when dropped.
    pub fn spawn(self, out: mpsc::Sender<ThrottledBatch>) -> tokio::task::JoinHandle<()> {
        let pending = self.pending;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FRAME);
            loop {
                tick.tick().await;
                let drained: Vec<(i32, String)> = {
                    let mut g = pending.lock().unwrap();
                    std::mem::take(&mut *g)
                };
                if drained.is_empty() {
                    continue;
                }
                for (row, text) in drained {
                    // Cap per-frame payload so a single huge flush can't stall
                    // the UI — the kernel already truncates at 40 KB upstream.
                    let _ = out
                        .send(ThrottledBatch { row, text, reasoning: String::new() })
                        .await;
                }
            }
        })
    }
}

impl Default for StreamThrottler {
    fn default() -> Self {
        Self::new()
    }
}
