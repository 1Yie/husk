//! Prompt-cache warming (pi `cacheWarming`).
//!
//! A provider's prompt cache lives for a few minutes after the last request
//! that read or wrote it (`promptCache.short`/`long`, seconds). When a long
//! tool run, a slow approval, or the user reading the answer outlasts that,
//! the next request re-writes the whole prefix at full price.
//! [`WarmingProvider`] wraps the session provider, remembers the last real
//! request, and — when the expected saving beats the cost — replays it with a
//! one-token output cap shortly before the entry expires. The replay is
//! byte-identical up to the cap, so it reads (and so refreshes) the same
//! cache entry.
//!
//! Safety limits mirror pi: warming never runs more than an hour past the real
//! request that armed it (30 minutes once the turn has settled), a replay that
//! would land after expiry is skipped (it would be a full-price write), and a
//! new real request, a settled turn in `streaming` mode, or
//! [`WarmingControl::cancel`] disarms it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;

use crate::config::{ModelCost, PromptCache};
use crate::provider::{BoxStream, Capabilities, LlmProvider, ModelParams};
use crate::types::{ChatMessage, StreamChunk};

/// Warming never continues past this long after the real request that armed it.
const MAX_WARMING_AGE: Duration = Duration::from_secs(60 * 60);
/// Idle warming uses a shorter horizon — continuation is less likely with age.
const MAX_IDLE_WARMING_AGE: Duration = Duration::from_secs(30 * 60);
/// A refresh is sent only when it is expected to save at least this many dollars.
const MINIMUM_EXPECTED_SAVINGS: f64 = 0.05;
/// Chance a real request arrives before expiry while the session sits idle
/// (pi's measured constant).
const IDLE_CONTINUATION_PROBABILITY: f64 = 0.15;

/// When warming may run (pi `cacheWarming`): `off`, only while a turn is
/// running (`streaming`, the default), or also after it settles (`idle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmingMode {
    Off,
    Streaming,
    Idle,
}

impl WarmingMode {
    /// `HUSK_CACHE_WARMING` — `off` | `streaming` (default) | `idle`.
    pub fn from_env() -> Self {
        Self::parse(std::env::var("HUSK_CACHE_WARMING").ok().as_deref())
    }

    fn parse(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("off") => Self::Off,
            Some(v) if v.eq_ignore_ascii_case("idle") => Self::Idle,
            _ => Self::Streaming,
        }
    }
}

/// Refresh at 90% of the TTL while keeping at least ten seconds of margin.
/// `None` when the lifetime is too short to warm at all.
fn warming_delay(ttl: Duration) -> Option<Duration> {
    if ttl <= Duration::from_secs(10) {
        return None;
    }
    Some(ttl.mul_f64(0.9).min(ttl - Duration::from_secs(10)))
}

/// Expected dollars saved by one refresh of a `prompt_tokens` prefix:
/// `p · (miss − hit) − warm`. `None` when the model declares no prices.
fn expected_savings(cost: &ModelCost, prompt_tokens: u64, idle: bool) -> Option<f64> {
    if prompt_tokens == 0 {
        return None;
    }
    let hit = cost.price(prompt_tokens, 0, 0, prompt_tokens, 0);
    let miss = if cost.cache_write.unwrap_or(0.0) > 0.0 {
        cost.price(prompt_tokens, 0, 0, 0, prompt_tokens)
    } else {
        cost.price(prompt_tokens, prompt_tokens, 0, 0, 0)
    };
    if hit <= 0.0 && miss <= 0.0 {
        return None;
    }
    let warm = cost.price(prompt_tokens, 0, 1, prompt_tokens, 0);
    let p = if idle {
        IDLE_CONTINUATION_PROBABILITY
    } else {
        1.0
    };
    Some(p * (miss - hit).max(0.0) - warm)
}

/// The real request being kept warm.
#[derive(Clone)]
struct Armed {
    model: String,
    messages: Vec<ChatMessage>,
    tools: Option<serde_json::Value>,
    temperature: f32,
    reasoning_effort: Option<String>,
    params: ModelParams,
    armed_at: Instant,
    /// Prompt tokens the provider reported for the request (0 until `Done`).
    prompt_tokens: u64,
}

#[derive(Default)]
struct State {
    armed: Option<Armed>,
    /// Bumped by every real request and every disarm — a timer holding a
    /// stale generation belongs to a superseded request and exits.
    generation: u64,
    /// A real request is streaming right now.
    in_flight: bool,
    /// The session's turn finished (set by [`WarmingControl::settle`]).
    settled: bool,
}

impl State {
    fn disarm(&mut self) {
        self.generation += 1;
        self.armed = None;
    }
}

/// The host's side of warming: tells the warmer when the turn settles and
/// when the conversation changed under it. Cheap to clone; a no-op when the
/// session's provider is not warmed.
#[derive(Clone, Default)]
pub struct WarmingControl {
    state: Arc<Mutex<State>>,
    mode: Arc<Mutex<Option<WarmingMode>>>,
}

impl WarmingControl {
    /// The turn finished. `streaming` mode stops here; `idle` keeps warming
    /// on the shorter idle horizon with the idle continuation odds.
    pub fn settle(&self) {
        let mode = *self.mode.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.settled = true;
        if mode != Some(WarmingMode::Idle) {
            st.disarm();
        }
    }

    /// The history no longer matches the armed request (compaction, undo,
    /// retry, clear, shutdown) — stop warming it.
    pub fn cancel(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .disarm();
    }
}

struct Shared {
    inner: Arc<dyn LlmProvider>,
    /// The instance warm replays go through. Kept apart from `inner` on
    /// purpose: a replay still publishes its cache-write count into whatever
    /// usage observer its provider carries, and the session reads that counter
    /// asynchronously — a replay landing in that window would overwrite the
    /// foreground round's figure. The host passes an observer-less twin.
    replay: Arc<dyn LlmProvider>,
    /// The model the session runs. Requests for other models (a fallback, a
    /// background summary) never arm warming.
    model: String,
    ttl: Duration,
    cost: ModelCost,
    control: WarmingControl,
}

/// Session provider wrapper that keeps the last request's prompt cache warm.
pub struct WarmingProvider(Arc<Shared>);

impl WarmingProvider {
    /// Wrap `inner` for `model`, replaying warm-up requests through `replay`.
    /// Returns `inner` unchanged when warming is off, prompt caching is off
    /// (`HUSK_CACHE_RETENTION=none`), or the model declares no lifetime for the
    /// active tier or no prices to weigh it by.
    ///
    /// `replay` is normally the same provider without the session's usage
    /// observer; passing `inner.clone()` is correct for a host that keeps no
    /// such counter.
    pub fn wrap(
        inner: Arc<dyn LlmProvider>,
        replay: Arc<dyn LlmProvider>,
        model: &str,
        prompt_cache: Option<&PromptCache>,
        cost: Option<&ModelCost>,
        control: &WarmingControl,
    ) -> Arc<dyn LlmProvider> {
        let mode = WarmingMode::from_env();
        let retention = std::env::var("HUSK_CACHE_RETENTION").unwrap_or_default();
        let retention = retention.trim();
        let long = retention.eq_ignore_ascii_case("long");
        let ttl = prompt_cache.and_then(|p| p.lifetime(long));
        let (Some(ttl), Some(cost)) = (ttl, cost) else {
            return inner;
        };
        if mode == WarmingMode::Off || retention.eq_ignore_ascii_case("none") {
            return inner;
        }
        *control.mode.lock().unwrap_or_else(|e| e.into_inner()) = Some(mode);
        Arc::new(Self(Arc::new(Shared {
            inner,
            replay,
            model: model.to_string(),
            ttl: Duration::from_secs(ttl),
            cost: cost.clone(),
            control: control.clone(),
        })))
    }
}

/// Sleep until the next refresh, replay, repeat — until superseded, expired,
/// or no longer worth it.
async fn warm_loop(shared: Arc<Shared>, generation: u64) {
    let Some(mut delay) = warming_delay(shared.ttl) else {
        return;
    };
    loop {
        tokio::time::sleep(delay).await;
        let armed = {
            let st = shared
                .control
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if st.generation != generation || st.in_flight {
                return;
            }
            let Some(armed) = st.armed.clone() else {
                return;
            };
            let horizon = if st.settled {
                MAX_IDLE_WARMING_AGE
            } else {
                MAX_WARMING_AGE
            };
            if armed.armed_at.elapsed() >= horizon {
                return;
            }
            let worth = expected_savings(&shared.cost, armed.prompt_tokens, st.settled)
                .is_some_and(|s| s >= MINIMUM_EXPECTED_SAVINGS);
            if !worth {
                return;
            }
            armed
        };
        let mut params = armed.params.clone();
        params.max_tokens = Some(1);
        let sent = Instant::now();
        // Best effort: an error never affects the session.
        if let Ok(mut stream) = shared
            .replay
            .chat_stream(
                &armed.model,
                &armed.messages,
                armed.tools.clone(),
                armed.temperature,
                armed.reasoning_effort.as_deref(),
                &params,
            )
            .await
        {
            while stream.next().await.is_some() {}
        }
        // The refresh itself restarted the entry's clock.
        delay = warming_delay(shared.ttl)
            .unwrap_or_default()
            .saturating_sub(sent.elapsed());
    }
}

#[async_trait]
impl LlmProvider for WarmingProvider {
    fn id(&self) -> &'static str {
        self.0.inner.id()
    }

    fn capabilities(&self) -> Capabilities {
        self.0.inner.capabilities()
    }

    fn native_tool_calls(&self) -> bool {
        self.0.inner.native_tool_calls()
    }

    async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        temperature: f32,
        reasoning_effort: Option<&str>,
        params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let shared = &self.0;
        // A real request supersedes the armed one: its own write is fresher.
        let generation = {
            let mut st = shared
                .control
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            st.disarm();
            st.in_flight = true;
            st.settled = false;
            if model == shared.model {
                st.armed = Some(Armed {
                    model: model.to_string(),
                    messages: messages.to_vec(),
                    tools: tools.clone(),
                    temperature,
                    reasoning_effort: reasoning_effort.map(str::to_string),
                    params: params.clone(),
                    armed_at: Instant::now(),
                    prompt_tokens: 0,
                });
            }
            st.generation
        };
        let result = shared
            .inner
            .chat_stream(
                model,
                messages,
                tools,
                temperature,
                reasoning_effort,
                params,
            )
            .await;
        let stream = match result {
            Ok(s) => s,
            Err(e) => {
                let mut st = shared
                    .control
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if st.generation == generation {
                    st.in_flight = false;
                    st.disarm();
                }
                return Err(e);
            }
        };
        let on_chunk = shared.clone();
        let mapped = stream.map(move |item| {
            if let Ok(StreamChunk::Done { prompt_tokens, .. }) = &item {
                let mut st = on_chunk
                    .control
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if st.generation == generation {
                    if let Some(a) = st.armed.as_mut() {
                        a.prompt_tokens = prompt_tokens.unwrap_or(0) as u64;
                    }
                }
            }
            item
        });
        // When the stream is fully consumed, the entry's clock starts: arm the
        // timer (no-op if a newer request or a cancel got there first).
        let on_end = shared.clone();
        let finished = futures::stream::once(async move {
            let armed = {
                let mut st = on_end
                    .control
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if st.generation == generation {
                    st.in_flight = false;
                }
                st.generation == generation && st.armed.is_some()
            };
            if armed {
                if let Ok(rt) = tokio::runtime::Handle::try_current() {
                    rt.spawn(warm_loop(on_end, generation));
                }
            }
            None
        })
        .filter_map(futures::future::ready);
        Ok(Box::pin(mapped.chain(finished)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn delay_keeps_ten_seconds_of_margin() {
        assert_eq!(warming_delay(Duration::from_secs(5)), None);
        assert_eq!(
            warming_delay(Duration::from_secs(300)),
            Some(Duration::from_secs(270))
        );
        assert_eq!(
            warming_delay(Duration::from_secs(60)),
            Some(Duration::from_secs(50))
        );
    }

    #[test]
    fn mode_parses_env_values() {
        assert_eq!(WarmingMode::parse(None), WarmingMode::Streaming);
        assert_eq!(WarmingMode::parse(Some("OFF")), WarmingMode::Off);
        assert_eq!(WarmingMode::parse(Some(" idle")), WarmingMode::Idle);
    }

    fn sonnet() -> ModelCost {
        ModelCost {
            input: Some(3.0),
            output: Some(15.0),
            cache_read: Some(0.3),
            cache_write: Some(3.75),
            tiers: Vec::new(),
        }
    }

    #[test]
    fn savings_follow_pi_economics() {
        // 100k-token prefix: miss $0.375, hit $0.03, warm $0.030015.
        let s = expected_savings(&sonnet(), 100_000, false).unwrap();
        assert!((s - (0.375 - 0.03 - 0.030015)).abs() < 1e-9, "{s}");
        // Idle discounts the miss by the continuation probability.
        let idle = expected_savings(&sonnet(), 100_000, true).unwrap();
        assert!(idle < MINIMUM_EXPECTED_SAVINGS, "{idle}");
        assert_eq!(expected_savings(&sonnet(), 0, false), None);
        assert_eq!(expected_savings(&ModelCost::default(), 1000, false), None);
    }

    /// Counts calls and records each one's `max_tokens`.
    struct Counting {
        calls: AtomicUsize,
        caps: Mutex<Vec<Option<u32>>>,
    }

    #[async_trait]
    impl LlmProvider for Counting {
        fn id(&self) -> &'static str {
            "counting"
        }

        async fn chat_stream(
            &self,
            _model: &str,
            _messages: &[ChatMessage],
            _tools: Option<serde_json::Value>,
            _temperature: f32,
            _reasoning_effort: Option<&str>,
            params: &ModelParams,
        ) -> anyhow::Result<BoxStream<StreamChunk>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.caps.lock().unwrap().push(params.max_tokens);
            Ok(Box::pin(futures::stream::iter([Ok(StreamChunk::Done {
                prompt_tokens: Some(100_000),
                completion_tokens: Some(1),
                cached_tokens: Some(0),
            })])))
        }
    }

    fn warmed(control: &WarmingControl) -> (Arc<Counting>, Arc<dyn LlmProvider>) {
        let inner = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            caps: Mutex::new(Vec::new()),
        });
        let cache = PromptCache {
            short: Some(300),
            long: None,
        };
        let p = WarmingProvider::wrap(
            inner.clone(),
            inner.clone(),
            "m",
            Some(&cache),
            Some(&sonnet()),
            control,
        );
        (inner, p)
    }

    async fn real_request(p: &Arc<dyn LlmProvider>) {
        let mut s = p
            .chat_stream(
                "m",
                &[ChatMessage::user("q")],
                None,
                0.0,
                None,
                &ModelParams::default(),
            )
            .await
            .unwrap();
        while s.next().await.is_some() {}
    }

    /// While the turn runs (a long tool call), the request is replayed with a
    /// one-token cap just before the 300s entry would expire.
    #[tokio::test(start_paused = true)]
    async fn replays_before_expiry_while_the_turn_runs() {
        let control = WarmingControl::default();
        let (inner, p) = warmed(&control);
        real_request(&p).await;
        tokio::time::sleep(Duration::from_secs(269)).await;
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
        assert_eq!(inner.caps.lock().unwrap()[1], Some(1));
    }

    /// `streaming` mode stops once the turn settles; a cancel stops any mode.
    #[tokio::test(start_paused = true)]
    async fn settle_and_cancel_disarm() {
        let control = WarmingControl::default();
        let (inner, p) = warmed(&control);
        real_request(&p).await;
        control.settle();
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);

        real_request(&p).await;
        control.cancel();
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
    }

    /// No lifetime or no prices → the provider is returned unwrapped.
    #[test]
    fn unwarmable_models_pass_through() {
        let inner: Arc<dyn LlmProvider> = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            caps: Mutex::new(Vec::new()),
        });
        let control = WarmingControl::default();
        let p = WarmingProvider::wrap(
            inner.clone(),
            inner.clone(),
            "m",
            None,
            Some(&sonnet()),
            &control,
        );
        assert!(Arc::ptr_eq(&p, &inner));
        let cache = PromptCache {
            short: Some(300),
            long: None,
        };
        let p = WarmingProvider::wrap(
            inner.clone(),
            inner.clone(),
            "m",
            Some(&cache),
            None,
            &control,
        );
        assert!(Arc::ptr_eq(&p, &inner));
    }
}
