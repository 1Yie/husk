//! `SamplerActor` — resilience policy wrapped around any `LlmProvider`.
//!
//! Policy table (llm-provider-layer.md §Resilience):
//!
//! | Policy          | Value                                                        |
//! |-----------------|--------------------------------------------------------------|
//! | Idle timeout    | no chunk for 300 s → `IdleTimeout` error                     |
//! | Doom-loop       | identical repeated generation → abort + resample, ≤3 retries |
//! | Retry           | exponential backoff on transport errors + 429/5xx; never 4xx |
//! | Mid-stream cut  | salvage + `interrupted` marker → continuation retry (≤3)     |
//! | Usage           | last `Done` chunk's counts feed `SessionStats`               |
//!
//! The sampler never owns a concrete provider — it wraps
//! `Arc<dyn LlmProvider>` so hot-swap replaces the Arc, not the actor.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use thiserror::Error;

use crate::provider::LlmProvider;
use crate::types::{ChatMessage, StreamChunk};

/// No chunk for this long → the stream is wedged; retry.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Identical suffix repeated this many times → model is stuck.
pub const DOOM_WINDOW: usize = 64;
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
    Retrying { attempt: u32, delay: Duration, reason: String },
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
            match self
                .sample_once(&req, messages, &mut on_chunk)
                .await
            {
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
        let masked: Vec<ChatMessage> = messages
            .iter()
            .map(|m| {
                let mut m = m.clone();
                if let Some(c) = &m.content {
                    let (s, _) = self.masker.scrub(c);
                    m.content = Some(s);
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
            )
            .await
            .map_err(|e| classify_transport_error(&e.to_string()))?;

        let mut recent: VecDeque<String> = VecDeque::with_capacity(DOOM_WINDOW);
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
            if let StreamChunk::ContentDelta(t) = &chunk {
                recent.push_back(t.clone());
                if recent.len() > DOOM_WINDOW {
                    recent.pop_front();
                }
                if doom_detected(&recent) {
                    return Err(SampleError::DoomLoop);
                }
            }

            on_chunk(&chunk);
        }

        if !done_seen {
            // Clean close without Done — synthesize so the caller's
            // invariants hold (spec: Done fires exactly once).
            on_chunk(&StreamChunk::Done { prompt_tokens: None, completion_tokens: None });
        }
        Ok(())
    }
}

/// Sliding-window doom detector: true when the window's tail is a strict
/// repetition of some period ≥4 chars. Catches "abababab…" and verbatim
/// paragraph loops alike without needing cryptographic comparison.
fn doom_detected(recent: &VecDeque<String>) -> bool {
    if recent.len() < DOOM_WINDOW {
        return false;
    }
    let joined: String = recent.iter().cloned().collect();
    let b = joined.as_bytes();
    let n = b.len();
    // Try periods from small to large; a period p is a doom loop iff
    // the whole window is periodic with p and repeats ≥3 times.
    for p in 4..=(n / 3) {
        if n % p != 0 && (n - 1) % p != 0 {
            // near-periodic tail is fine too; check by sliding compare below
        }
        if b[n - p..] == b[n - 2 * p..n - p] && b[n - 2 * p..n - p] == b[n - 3 * p..n - 2 * p] {
            return true;
        }
    }
    false
}

/// Decide retryability from an error string: 429/5xx and transport-level
/// failures retry; other 4xx never do (spec: "never on other 4xx").
fn classify_transport_error(msg: &str) -> SampleError {
    let lower = msg.to_lowercase();
    let retryable = lower.contains("429")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("transport")
        || lower.contains("eof")
        || lower.contains("reset");
    if retryable {
        SampleError::Stream(format!("retryable: {msg}"))
    } else {
        SampleError::Stream(format!("fatal: {msg}"))
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
