//! Shared SSE helpers on `eventsource-stream`.
//!
//! `data_lines` turns a raw `reqwest` byte stream into a stream of
//! `data:`-payload strings, tolerating the two close conventions: the
//! `[DONE]` sentinel (OpenAI/xAI/DeepSeek) and a clean connection close
//! (Ollama/vLLM). The sentinel arrives as `Some("[DONE]")` like any other
//! payload — the *adapter* decides it's terminal.

use anyhow::anyhow;
use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt};

use crate::provider::BoxStream;

/// SSE `data:` payloads as a stream of strings. Malformed frames surface as
/// `Err` items (not stream termination) so the sampler can decide.
pub fn data_lines<S>(byte_stream: S) -> BoxStream<String>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    let events = byte_stream.eventsource();
    let mapped = events.filter_map(|res| async move {
        match res {
            Ok(ev) if ev.data.is_empty() => None, // heartbeat / comment frames
            Ok(ev) => Some(Ok(ev.data)),
            Err(e) => Some(Err(anyhow!("SSE parse error: {e}"))),
        }
    });
    Box::pin(mapped)
}
