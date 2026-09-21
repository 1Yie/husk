//! `MockProvider` — deterministic scripted provider for headless tests.
//!
//! Engine, compaction, and tool-loop tests run with zero network: queue
//! scripts of `StreamChunk`s, one script per `chat_stream` call, optional
//! per-chunk latency.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::StreamExt;

use crate::provider::{BoxStream, LlmProvider};
use crate::types::{ChatMessage, StreamChunk};

/// Replays scripted chunk lists. Each `chat_stream` call pops one script
/// (FIFO); when scripts run out it returns an error chunk, making test bugs
/// loud instead of silently looping.
pub struct MockProvider {
    scripts: Mutex<VecDeque<Vec<StreamChunk>>>,
    latency: Duration,
    /// Captured requests for assertions: (model, message_count, tools_present).
    pub calls: Mutex<Vec<(String, usize, bool)>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            scripts: Mutex::new(VecDeque::new()),
            latency: Duration::ZERO,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Queue one response script. Chainable.
    pub fn with_script(self, chunks: Vec<StreamChunk>) -> Self {
        self.scripts.lock().unwrap().push_back(chunks);
        self
    }

    pub fn push_script(&self, chunks: Vec<StreamChunk>) {
        self.scripts.lock().unwrap().push_back(chunks);
    }

    /// Per-chunk delivery delay — used to exercise throttling/steering paths.
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Convenience: script a simple text answer ending in `Done`.
    pub fn script_text(&self, text: &str) {
        let mut chunks: Vec<StreamChunk> = text
            .chars()
            .collect::<Vec<_>>()
            .chunks(8)
            .map(|c| StreamChunk::ContentDelta(c.iter().collect()))
            .collect();
        chunks.push(StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None });
        self.push_script(chunks);
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn id(&self) -> &'static str {
        "mock"
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        _temperature: f32,
        _reasoning_effort: Option<&str>,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        self.calls.lock().unwrap().push((
            model.to_string(),
            messages.len(),
            tools.is_some(),
        ));

        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("MockProvider: no script queued for this call"))?;

        let latency = self.latency;
        let stream = futures::stream::iter(script).then(move |chunk| async move {
            if latency > Duration::ZERO {
                tokio::time::sleep(latency).await;
            }
            Ok(chunk)
        });
        Ok(Box::pin(stream))
    }
}
