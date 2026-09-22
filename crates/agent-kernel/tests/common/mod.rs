//! `ScriptedProvider` — deterministic provider for headless tests.
//!
//! Queue scripts of `StreamChunk`s, one script per `chat_stream` call
//! (FIFO). When scripts run out the call errors, so test bugs are loud.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::StreamExt;

use agent_llm::provider::{BoxStream, LlmProvider};
use agent_llm::types::{ChatMessage, StreamChunk};

pub struct ScriptedProvider {
    scripts: Mutex<VecDeque<Vec<StreamChunk>>>,
    latency: Duration,
    /// Captured requests for assertions: (model, message_count, tools_present).
    pub calls: Mutex<Vec<(String, usize, bool)>>,
}

impl ScriptedProvider {
    pub fn new() -> Self {
        Self {
            scripts: Mutex::new(VecDeque::new()),
            latency: Duration::ZERO,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with_script(self, chunks: Vec<StreamChunk>) -> Self {
        self.scripts.lock().unwrap().push_back(chunks);
        self
    }

    pub fn push_script(&self, chunks: Vec<StreamChunk>) {
        self.scripts.lock().unwrap().push_back(chunks);
    }

    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// Script a simple text answer ending in `Done`.
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

impl Default for ScriptedProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn id(&self) -> &'static str {
        "scripted"
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        _temperature: f32,
        _reasoning_effort: Option<&str>,
        _params: &agent_llm::ModelParams,
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
            .ok_or_else(|| anyhow!("ScriptedProvider: no script queued for this call"))?;
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
