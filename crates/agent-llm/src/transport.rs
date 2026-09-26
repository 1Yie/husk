//! Shared HTTP/SSE transport — the wire mechanics every adapter used to
//! repeat: one pooled reqwest client, `Accept: text/event-stream`, a JSON
//! POST body, the non-2xx gate, and the Done-exactly-once guarantee.
//!
//! Adapters keep what differs — URL construction, request body, auth
//! header, event→chunk mapping. This file owns what doesn't.

use anyhow::{anyhow, Context};
use futures::StreamExt;

use crate::provider::BoxStream;
use crate::types::StreamChunk;

/// One pooled client + provider-static headers. Cheap to construct per
/// provider instance; the pool (`max_idle_per_host=4`) keeps SSE
/// connections warm across turns.
pub struct Transport {
    client: reqwest::Client,
    extra_headers: Vec<(String, String)>,
}

impl Transport {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(4)
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            client,
            extra_headers: Vec::new(),
        })
    }

    /// A provider-static header applied to every request (e.g.
    /// `HTTP-Referer` on OpenRouter-style shims).
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((key.into(), value.into()));
        self
    }

    /// POST `body` as JSON to `url` with the SSE accept header and provider
    /// extras applied — the adapter then adds auth and protocol headers on
    /// the returned builder (`bearer_auth`, `x-api-key`, `x-goog-api-key`,
    /// `anthropic-version`, …) before handing it to [`Self::send_sse`].
    pub fn post(&self, url: &str, body: &serde_json::Value) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(url)
            .header("Accept", "text/event-stream")
            .json(body);
        for (k, v) in &self.extra_headers {
            req = req.header(k, v);
        }
        req
    }

    /// Send → status gate → `data:` payload stream.
    ///
    /// `AGENT_DUMP_REQ` dumps the outbound body to stderr (labelled by
    /// `what`) so a silently-empty reply can be correlated to the exact
    /// request shape. A non-2xx carries the first 512 chars of the error
    /// body — enough for the upstream's own message, short of a leaked
    /// HTML error page. The status is emitted as a parseable
    /// `provider HTTP <code>` prefix so the sampler classifies retries off
    /// the real code instead of guessing from body text.
    pub async fn send_sse(
        &self,
        req: reqwest::RequestBuilder,
        body: &serde_json::Value,
        what: &'static str,
    ) -> anyhow::Result<BoxStream<String>> {
        if std::env::var("AGENT_DUMP_REQ").is_ok() {
            eprintln!(
                "\n===REQ({what})===\n{}\n===/REQ===",
                serde_json::to_string(body).unwrap_or_default()
            );
        }
        let resp = req.send().await.context(what)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "provider HTTP {}: {}",
                status.as_u16(),
                clip_chars(&text, 512)
            ));
        }
        Ok(crate::sse::data_lines(resp.bytes_stream()))
    }
}

/// First `max` CHARS of `s` — never a mid-character cut. A byte slice
/// (`&s[..n]`) panics when the cut lands inside a multi-byte char, and this
/// sits on the error path of every provider: a non-ASCII error page (中文
/// upstream message, mojibake HTML) would take the whole process down under
/// `panic = "abort"`.
fn clip_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Enforces the layer's terminal invariant — `chat_stream` yields `0..N` deltas then
/// exactly one `Done`, even when a backend closes the SSE connection without a
/// terminal event ([DONE] sentinel, `response.completed`, `message_stop` and a
/// trailing usage chunk all signal differently). The flag is a shared `AtomicBool`,
/// not a `Copy` bool: an `async move` fallback closure would snapshot `false` at
/// construction and emit a duplicate `Done` on every stream.
#[derive(Clone, Default)]
pub struct DoneGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl DoneGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call for every chunk the mapper emits — a `Done` passing through
    /// marks the flag so `finish` stays silent.
    pub fn observe(&self, chunk: &StreamChunk) {
        if matches!(chunk, StreamChunk::Done { .. }) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Append `make()`'s chunk iff the wire never emitted `Done`. The flag
    /// is read at poll time (the `once` runs only after `stream` is
    /// exhausted), so it sees the true terminal state — not the
    /// construction-time snapshot that broke the hand-rolled versions.
    pub fn finish<S, F>(self, stream: S, make: F) -> BoxStream<StreamChunk>
    where
        S: futures::Stream<Item = anyhow::Result<StreamChunk>> + Send + 'static,
        F: FnOnce() -> StreamChunk + Send + 'static,
    {
        let tail = futures::stream::once(async move {
            if self.0.load(std::sync::atomic::Ordering::Relaxed) {
                None
            } else {
                Some(Ok(make()))
            }
        })
        .filter_map(|x| async move { x });
        Box::pin(stream.chain(tail))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn done() -> StreamChunk {
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        }
    }

    #[tokio::test]
    async fn finish_synthesizes_done_when_wire_omits_it() {
        let guard = DoneGuard::new();
        let src = futures::stream::iter(vec![Ok(StreamChunk::ContentDelta("hi".into()))]);
        let out: Vec<_> = guard.finish(src, done).collect().await;
        assert!(matches!(out.last(), Some(Ok(StreamChunk::Done { .. }))));
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn finish_stays_silent_after_wire_done() {
        let guard = DoneGuard::new();
        let flag = guard.clone();
        let src = futures::stream::iter(vec![
            Ok(StreamChunk::ContentDelta("hi".into())),
            Ok(StreamChunk::Done {
                prompt_tokens: Some(1),
                completion_tokens: Some(2),
                cached_tokens: None,
            }),
        ])
        .map(move |c| {
            if let Ok(ref ch) = c {
                flag.observe(ch);
            }
            c
        });
        let out: Vec<_> = guard.finish(src, done).collect().await;
        // Exactly one Done — the wire's own.
        let dones = out
            .iter()
            .filter(|c| matches!(c, Ok(StreamChunk::Done { .. })))
            .count();
        assert_eq!(dones, 1);
        assert_eq!(out.len(), 2);
    }

    /// The error-body clip used to be `&text[..512]` — a byte cut that panics
    /// when it lands inside a multi-byte char, and this runs on the error path
    /// of every provider under `panic = "abort"`.
    #[test]
    fn clip_chars_never_cuts_mid_character() {
        // '中' is 3 bytes: a byte slice at 512 would split it.
        let s = "中".repeat(600);
        let clipped = clip_chars(&s, 512);
        assert_eq!(clipped.chars().count(), 512);
        assert_eq!(clipped.len(), 512 * 3);

        // Mixed widths — a CJK char sitting exactly on the boundary.
        let mixed = format!("{}{}", "x".repeat(511), "中");
        assert_eq!(clip_chars(&mixed, 512), mixed);

        // Short strings pass through untouched.
        assert_eq!(clip_chars("ok", 512), "ok");
    }
}
