//! `SamplerActor` — resilience policy wrapped around any `LlmProvider`.
//!
//! | Policy         | Value                                                        |
//! |----------------|--------------------------------------------------------------|
//! | Idle timeout   | no chunk for 300 s → `IdleTimeout` error                     |
//! | Doom-loop      | identical repeated generation → abort + resample, ≤3 retries |
//! | Retry          | exponential backoff on transport errors + 429/5xx, never 4xx |
//! | Mid-stream cut | retry resends the same messages → provider regenerates;      |
//! |                | the caller drops attempt-scoped buffers on `Retrying`        |
//! | Usage          | the last `Done` chunk's counts feed `SessionStats`            |
//!
//! The sampler never owns a concrete provider — it wraps `Arc<dyn LlmProvider>`,
//! so a hot-swap replaces the Arc, not the actor.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use thiserror::Error;

use crate::provider::LlmProvider;
use crate::provider::ModelParams;
use crate::types::{ChatMessage, StreamChunk};

/// No chunk for this long → the stream is wedged; retry.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Identical generation repeated across this many recent BYTES → model is
/// stuck. Byte-bounded, not chunk-bounded: providers chunk deltas anywhere
/// from 1 char to whole paragraphs, so a fixed chunk count made the analysis
/// window (and its period bounds) a function of the backend's segmentation.
pub const DOOM_WINDOW_BYTES: usize = 512;
/// Retry ceiling for doom loops and transport errors.
pub const MAX_RETRIES: u32 = 3;
/// Backoff base for transport/429/5xx retries.
pub const RETRY_BASE: Duration = Duration::from_millis(500);

#[derive(Debug, Error)]
pub enum SampleError {
    #[error("stream idle for {}s", IDLE_TIMEOUT.as_secs())]
    IdleTimeout,

    #[error("doom loop detected: identical generation repeated")]
    DoomLoop,

    #[error("provider error after {attempts} attempts: {message}")]
    Exhausted { attempts: u32, message: String },

    #[error("stream error: {0}")]
    Stream(String),
}

/// Events the sampler emits *besides* model chunks — surfaced to the UI as
/// `system` messages so degrade is visible, never silent.
#[derive(Debug, Clone)]
pub enum SamplerEvent {
    /// Retry k of N scheduled after `delay`.
    Retrying {
        attempt: u32,
        delay: Duration,
        reason: String,
    },
    /// Provider failed permanently; the session should degrade or abort.
    Failed(String),
}

/// Stateless resilience wrapper around one provider at a time.
/// `set_provider` hot-swaps for the *next* call — in-flight streams finish.
pub struct Sampler {
    provider: Arc<dyn LlmProvider>,
    /// Egress masker — scrubs resolved secrets from outbound request bodies.
    masker: crate::masking::EgressMasker,
}

#[derive(Debug, Clone, Copy)]
pub struct SampleRequest<'a> {
    pub model: &'a str,
    pub temperature: f32,
    pub tools: Option<&'a serde_json::Value>,
    pub reasoning_effort: Option<&'a str>,
    /// Per-model wire settings (`maxTokens`, `samplingParams`, model-level
    /// `compat`) — resolved from config by the caller, forwarded to the
    /// adapter. Use [`ModelParams::EMPTY`] when the model declares nothing.
    pub params: &'a ModelParams,
}

impl Sampler {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            masker: crate::masking::EgressMasker::new(vec![]),
        }
    }

    /// Install the resolved secrets for egress masking — called once config
    /// resolves `env:`/`keyring:` handles into real key material.
    pub fn set_masker(&mut self, secrets: Vec<String>) {
        self.masker = crate::masking::EgressMasker::new(secrets);
    }

    /// Hot-swap: next call uses the new provider. In-flight calls unaffected.
    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
    }

    pub fn provider(&self) -> Arc<dyn LlmProvider> {
        self.provider.clone()
    }

    /// One full request with retry/doom-loop policy. Calls `on_chunk` per
    /// chunk and `on_event` for policy events; returns the assembled text.
    ///
    /// Chunk ordering guarantee: `Done` reaches `on_chunk` exactly once per
    /// attempt, so the caller treats `Err` from this function as the attempt
    /// boundary, not a missing `Done`.
    pub async fn sample<F, G>(
        &self,
        req: SampleRequest<'_>,
        messages: &[ChatMessage],
        mut on_chunk: F,
        mut on_event: G,
    ) -> Result<(), SampleError>
    where
        F: FnMut(&StreamChunk),
        G: FnMut(&SamplerEvent),
    {
        let mut attempt = 0u32;
        loop {
            match self.sample_once(&req, messages, &mut on_chunk).await {
                Ok(()) => return Ok(()),
                Err(e) if !e.retryable() => return Err(e),
                Err(e) => {
                    attempt += 1;
                    if attempt >= MAX_RETRIES {
                        on_event(&SamplerEvent::Failed(e.to_string()));
                        return Err(SampleError::Exhausted {
                            attempts: attempt,
                            message: e.to_string(),
                        });
                    }
                    let delay = RETRY_BASE * 2u32.saturating_pow(attempt - 1);
                    on_event(&SamplerEvent::Retrying {
                        attempt,
                        delay,
                        reason: e.to_string(),
                    });
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// One attempt: drive the provider stream under idle-timeout and
    /// doom-loop supervision. **Egress masking** scrubs secret material out
    /// of the request messages before they hit the wire; **stream salvage**
    /// returns the partial text alongside the error so the caller can keep
    /// it visible + retry with a continuation prompt.
    async fn sample_once<F>(
        &self,
        req: &SampleRequest<'_>,
        messages: &[ChatMessage],
        on_chunk: &mut F,
    ) -> Result<(), SampleError>
    where
        F: FnMut(&StreamChunk),
    {
        // Egress mask: scrub secrets from outbound messages (defense in
        // depth — sandbox env sanitize is the first wall, this is second).
        // Scrub BOTH `content` and `tool_calls[].arguments` — a tool that
        // read a secret (`.env`, `cat ~/.aws/credentials`) puts it into the
        // assistant's arguments on the NEXT request, which is the most
        // common exfiltration path and was previously unmasked.
        let masked: Vec<ChatMessage> = messages
            .iter()
            .map(|m| {
                let mut m = m.clone();
                if let Some(c) = &m.content {
                    let (s, _) = self.masker.scrub(c);
                    m.content = Some(s);
                }
                if let Some(calls) = &mut m.tool_calls {
                    for call in calls.iter_mut() {
                        let (s, _) = self.masker.scrub(&call.arguments);
                        call.arguments = s;
                    }
                }
                m
            })
            .collect();

        let mut stream = self
            .provider
            .chat_stream(
                req.model,
                &masked,
                req.tools.cloned(),
                req.temperature,
                req.reasoning_effort,
                req.params,
            )
            .await
            .map_err(|e| classify_transport_error(&e.to_string()))?;

        let mut recent: VecDeque<String> = VecDeque::with_capacity(64);
        let mut recent_bytes = 0usize;
        let mut done_seen = false;

        loop {
            let next = tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await;
            let chunk = match next {
                Err(_) => return Err(SampleError::IdleTimeout),
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    let msg = e.to_string();
                    let err = classify_transport_error(&msg);
                    return Err(err);
                }
                Ok(Some(Ok(c))) => c,
            };

            if let StreamChunk::Done { .. } = &chunk {
                if done_seen {
                    continue; // provider double-fired Done; swallow
                }
                done_seen = true;
            }

            // Doom-loop: fold content deltas into a sliding window and check
            // for a repeated suffix period.
            // All three text-bearing variants count — a reasoning model can
            // doom-loop inside its thinking trace, or inside streamed tool
            // arguments, without a single ContentDelta ever arriving.
            let doom_text = match &chunk {
                StreamChunk::ContentDelta(t) | StreamChunk::ReasoningDelta(t) => Some(t.as_str()),
                StreamChunk::ToolCallDelta { args_delta, .. } => Some(args_delta.as_str()),
                _ => None,
            };
            if let Some(t) = doom_text {
                recent_bytes += t.len();
                recent.push_back(t.to_string());
                // Detect BEFORE trimming — the just-pushed chunk is what fills
                // the window past DOOM_WINDOW_BYTES; evicting first would
                // leave `joined` under the gate and the check would never run.
                if doom_detected(&recent) {
                    return Err(SampleError::DoomLoop);
                }
                while recent_bytes > DOOM_WINDOW_BYTES && recent.len() > 1 {
                    if let Some(old) = recent.pop_front() {
                        recent_bytes -= old.len();
                    }
                }
            }

            // A provider-level error INSIDE the stream (devin emits
            // `response.failed`/`error` events → `StreamChunk::Error`, e.g.
            // "internal_server_error … upstream error") is a real failure —
            // escalate to Err so the retry loop sees it. Forward the chunk
            // first so the UI still shows the message.
            if let StreamChunk::Error(msg) = &chunk {
                on_chunk(&chunk);
                return Err(classify_transport_error(msg));
            }

            on_chunk(&chunk);
        }

        if !done_seen {
            // Clean close without Done — synthesize so the caller's
            // invariants hold (spec: Done fires exactly once).
            on_chunk(&StreamChunk::Done {
                prompt_tokens: None,
                completion_tokens: None,
                cached_tokens: None,
            });
        }
        Ok(())
    }
}

/// Sliding-window doom detector: true when the window's tail is a strict
/// repetition of some period ≥4 chars. Catches "abababab…" and verbatim
/// paragraph loops alike without needing cryptographic comparison.
fn doom_detected(recent: &VecDeque<String>) -> bool {
    let joined: String = recent.iter().cloned().collect();
    let b = joined.as_bytes();
    let n = b.len();
    // Judge only a full window — the same bytes-per-window contract the old
    // 64-chunk gate approximated.
    if n < DOOM_WINDOW_BYTES {
        return false;
    }
    // Three equal tail slices alone isn't a doom loop — markdown lists,
    // repeated `className=` props, and `- ` bullets legitimately produce
    // identical 3× tails in a 64-char window on NORMAL output. A real doom
    // loop is the whole window being ~periodic: ≥85% of positions satisfy
    // b[i] == b[i-p] AND the tail shows ≥3 clean repeats. The 85% slack
    // tolerates the window boundary landing mid-repetition.
    for p in 4..=(n / 3) {
        // Tail must show ≥3 verbatim repeats first — the cheap check.
        if b[n - p..] != b[n - 2 * p..n - p] || b[n - 2 * p..n - p] != b[n - 3 * p..n - 2 * p] {
            continue;
        }
        let mut matches = 0usize;
        for i in p..n {
            if b[i] == b[i - p] {
                matches += 1;
            }
        }
        // matches/(n-p) ≥ 85% → the window is periodic with p.
        if matches * 20 >= (n - p) * 17 {
            return true;
        }
    }
    false
}

/// Decide retryability from an error string: 429/5xx and transport-level
/// failures retry; other 4xx never do (spec: "never on other 4xx").
fn classify_transport_error(msg: &str) -> SampleError {
    // `send_sse` prefixes real HTTP failures with `provider HTTP <code>` — a
    // parsed status beats substring guessing. Without it, a body that merely
    // CONTAINS a number ("max_tokens 15000", a request id with "500" in it)
    // was retried as if it were a 5xx.
    let code = msg
        .strip_prefix("provider HTTP ")
        .and_then(|s| s.split([':', ' ']).next())
        .and_then(|s| s.parse::<u16>().ok());
    let retryable = match code {
        Some(c) => c == 429 || c >= 500,
        // Fallback for failures that never carried a status — stream-chunk
        // errors, transport drops, gateway prose. Keyword-only on purpose:
        // bare digits in a free-text body are not a status code.
        None => {
            let lower = msg.to_lowercase();
            lower.contains("timeout")
                || lower.contains("timed out")
                || lower.contains("connection")
                || lower.contains("transport")
                || lower.contains("eof")
                || lower.contains("reset")
                // Gateway-style 5xx bodies that don't carry the numeric code —
                // devin reports `internal_server_error … upstream error`.
                || lower.contains("internal_server_error")
                || lower.contains("internal server error")
                || lower.contains("upstream error")
                || lower.contains("bad gateway")
                || lower.contains("service unavailable")
                || lower.contains("rate limit")
                || lower.contains("http 429")
                || lower.contains("http 5")
        }
    };
    if retryable {
        SampleError::Stream(format!("retryable: {msg}"))
    } else {
        SampleError::Stream(format!("fatal: {msg}"))
    }
}

/// Human-facing text for policy events — the engine sends these to the UI as
/// `SystemMessage` and persists them into history, so Debug output
/// (`Retrying { attempt: 1, delay: 500ms, … }`) is not acceptable there.
impl std::fmt::Display for SamplerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SamplerEvent::Retrying {
                attempt,
                delay,
                reason,
            } => write!(
                f,
                "上游响应中断 — {:.1}s 后重试（第 {attempt}/{MAX_RETRIES} 次）：{reason}",
                delay.as_secs_f32()
            ),
            SamplerEvent::Failed(reason) => write!(f, "上游持续不可用：{reason}"),
        }
    }
}

impl SampleError {
    /// Whether the sampler may retry this failure.
    fn retryable(&self) -> bool {
        match self {
            SampleError::IdleTimeout | SampleError::DoomLoop => true,
            SampleError::Stream(s) => s.starts_with("retryable:"),
            SampleError::Exhausted { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 400 whose body contains "500" is not a 5xx — the structured
    /// `provider HTTP <code>` prefix must beat body substring guessing,
    /// else a permanent failure burns the whole retry budget with backoff.
    #[test]
    fn classify_prefers_the_real_status_over_body_text() {
        // The reported false positive: a token-limit message containing 500.
        let e = classify_transport_error(
            "provider HTTP 400: {\"error\":\"max_tokens 15000 exceeds context\"}",
        );
        assert!(!e.retryable(), "a 400 must never retry: {e}");

        for code in [429u16, 500, 502, 503, 504] {
            let e = classify_transport_error(&format!("provider HTTP {code}: x"));
            assert!(e.retryable(), "HTTP {code} should retry");
        }
        for code in [400u16, 401, 403, 404, 422] {
            let e = classify_transport_error(&format!("provider HTTP {code}: x"));
            assert!(!e.retryable(), "HTTP {code} must not retry");
        }

        // No prefix → keyword fallback (in-stream errors, transport drops).
        assert!(classify_transport_error("internal_server_error: upstream error").retryable());
        assert!(classify_transport_error("connection reset by peer").retryable());
        // A bare number in free text is still not a status code.
        assert!(!classify_transport_error("model refused: 5000 tokens over budget").retryable());
    }

    /// The doom window is byte-bounded: providers chunk deltas anywhere from
    /// one char to whole paragraphs, so the detector must see the same ~512B
    /// regardless of segmentation.
    #[test]
    fn doom_window_is_byte_bounded() {
        // One fat chunk can fill the window on its own.
        let mut fat = VecDeque::new();
        fat.push_back("x".repeat(DOOM_WINDOW_BYTES + 64));
        assert!(doom_detected(&fat));

        // Many tiny chunks of the same repeated pattern.
        let mut many = VecDeque::new();
        for _ in 0..256 {
            many.push_back("ab".to_string());
        }
        assert!(doom_detected(&many));

        // A short window never fires.
        let mut small = VecDeque::new();
        small.push_back("ab".repeat(30));
        assert!(!doom_detected(&small));
    }
}
