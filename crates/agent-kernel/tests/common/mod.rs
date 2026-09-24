//! `ScriptedProvider` — deterministic provider for headless tests.
//!
//! Queue scripts of `StreamChunk`s, one script per `chat_stream` call
//! (FIFO). When scripts run out the call errors, so test bugs are loud.

// Shared by several test targets — each compiles this module separately, so
// "unused in THIS target" is not a defect.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::StreamExt;

use agent_llm::provider::{BoxStream, LlmProvider};
use agent_llm::types::{ChatMessage, StreamChunk};

pub struct ScriptedProvider {
    scripts: Mutex<VecDeque<Vec<StreamChunk>>>,
    /// Captured requests for assertions: (model, message_count, tools_present).
    pub calls: Mutex<Vec<(String, usize, bool)>>,
    /// Tool names in the `tools` schema of each request — how a test proves
    /// what the model could actually call (mode-scoped registries are
    /// otherwise a private engine detail).
    pub tool_names: Mutex<Vec<Vec<String>>>,
}

impl ScriptedProvider {
    pub fn new() -> Self {
        Self {
            scripts: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            tool_names: Mutex::new(Vec::new()),
        }
    }

    pub fn with_script(self, chunks: Vec<StreamChunk>) -> Self {
        self.scripts.lock().unwrap().push_back(chunks);
        self
    }

    pub fn push_script(&self, chunks: Vec<StreamChunk>) {
        self.scripts.lock().unwrap().push_back(chunks);
    }

    /// Script a simple text answer ending in `Done`.
    pub fn script_text(&self, text: &str) {
        let mut chunks: Vec<StreamChunk> = text
            .chars()
            .collect::<Vec<_>>()
            .chunks(8)
            .map(|c| StreamChunk::ContentDelta(c.iter().collect()))
            .collect();
        chunks.push(StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        });
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
        self.calls
            .lock()
            .unwrap()
            .push((model.to_string(), messages.len(), tools.is_some()));
        self.tool_names.lock().unwrap().push(
            tools
                .as_ref()
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t["function"]["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        );
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("ScriptedProvider: no script queued for this call"))?;
        let stream = futures::stream::iter(script).map(Ok);
        Ok(Box::pin(stream))
    }
}
