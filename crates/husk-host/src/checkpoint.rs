//! Durable recovery points for a turn that has not returned to its actor yet.
//!
//! The engine owns a mutable history for the entire turn. Its provider boundary
//! exposes each round's canonical context, while the returned stream exposes the
//! draft that has not entered that history yet. Keep both on disk; never depend
//! on Drop or a cooperative cancellation to save a running task.

use crate::context_breakdown::CONTEXT_BREAKDOWN;
use agent_kernel::channels::{UiReceiver, UiSink};
use agent_kernel::hooks::{AgentHook, HookChain};
use agent_llm::types::{now_ms, ChatMessage, Role, StreamChunk, ToolCall, ToolCallAssembler};
use agent_llm::{BoxStream, Capabilities, LlmProvider, ModelParams};
use agent_store::{SessionMeta, SessionStore};
use futures::Stream;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::oneshot;

const BARRIER: &str = "husk:checkpoint_barrier";

/// How often a streaming draft is persisted. A larger window trades a little
/// crash-recovery detail for far less serialization + page-cache churn: a
/// multi-megabyte history was being copied and rewritten four times a second.
/// Tool/approval boundaries still flush explicitly.
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(1000);
/// Retry cadence after a transient snapshot-write failure. Kept short and
/// separate from the draft cadence: a deleted/locked session must fail out
/// quickly instead of delaying shutdown by the streaming interval.
const CHECKPOINT_RETRY_DELAY: Duration = Duration::from_millis(250);
const INTERRUPTED_TURN: &str = "上次任务因会话意外中断而停止，已恢复到最后保存的进度。";
const INTERRUPTED_TOOL: &str = "会话在工具结果保存前中断，执行结果未知。继续前请检查实际状态，不要假定操作未执行，也不要直接重复有副作用的操作。";
/// UI-only extension accompanying a `Usage` event: the round's cache *write*
/// tokens, which the shared `UiEvent::Usage` shape cannot carry.
pub(crate) const USAGE_CACHE_WRITE: &str = "husk:usage_cache_write";

pub(crate) struct TurnCheckpoint {
    store: Arc<SessionStore>,
    ui: UiSink,
    state: Mutex<State>,
    /// At most one blocking writer runs; mutations coalesce into its next snapshot.
    writing: AtomicBool,
    dirty: AtomicBool,
    written: tokio::sync::Notify,
    relay: Mutex<Option<(UiReceiver, UiSink)>>,
    barriers: Mutex<std::collections::HashMap<u64, oneshot::Sender<()>>>,
    next_barrier: std::sync::atomic::AtomicU64,
    /// Provider-side cache-write counter for the current session.
    usage: Mutex<Option<Arc<agent_llm::UsageObserver>>>,
    /// Estimated composition of the latest request — relayed after the
    /// `Usage` event that request produced.
    breakdown: Mutex<Option<agent_store::ContextBreakdown>>,
}

struct State {
    revision: u64,
    generation: u64,
    active: Option<ActiveTurn>,
    /// Saving is given up for this turn (stale revision or repeated IO errors).
    failed: bool,
    /// Consecutive transient IO failures; the next save retries.
    failures: u8,
    /// The user has already been told about the current failure streak.
    notified: bool,
    settings: Option<LiveSettings>,
    /// Exact identities captured at the engine's before-tool boundary.
    dispatched: VecDeque<ToolCall>,
}

/// Transient save errors tolerated before the turn stops saving.
const MAX_SAVE_FAILURES: u8 = 5;

struct LiveSettings {
    permissions: Arc<std::sync::RwLock<agent_kernel::permissions::PermissionGate>>,
    agent_mode: Arc<std::sync::RwLock<String>>,
    thinking: Arc<std::sync::RwLock<Option<String>>>,
}

struct ActiveTurn {
    history: Vec<ChatMessage>,
    request: Vec<ChatMessage>,
    meta: SessionMeta,
}

struct RelayCleanup {
    checkpoint: std::sync::Weak<TurnCheckpoint>,
    forward: UiSink,
}

impl Drop for RelayCleanup {
    fn drop(&mut self) {
        if let Some(checkpoint) = self.checkpoint.upgrade() {
            checkpoint.ui.close();
            checkpoint
                .barriers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
        self.forward.close();
    }
}

impl TurnCheckpoint {
    pub(crate) fn new(
        store: Arc<SessionStore>,
        revision: u64,
        ui: UiSink,
        events: UiReceiver,
        forward: UiSink,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            ui,
            relay: Mutex::new(Some((events, forward))),
            barriers: Mutex::new(std::collections::HashMap::new()),
            next_barrier: std::sync::atomic::AtomicU64::new(0),
            usage: Mutex::new(None),
            breakdown: Mutex::new(None),
            writing: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            written: tokio::sync::Notify::new(),
            state: Mutex::new(State {
                revision,
                generation: 0,
                active: None,
                failed: false,
                failures: 0,
                notified: false,
                settings: None,
                dispatched: VecDeque::new(),
            }),
        })
    }

    pub(crate) fn set_settings(
        &self,
        permissions: Arc<std::sync::RwLock<agent_kernel::permissions::PermissionGate>>,
        agent_mode: Arc<std::sync::RwLock<String>>,
        thinking: Arc<std::sync::RwLock<Option<String>>>,
    ) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .settings = Some(LiveSettings {
            permissions,
            agent_mode,
            thinking,
        });
    }

    /// Share the provider's cache-write counter with the relay.
    pub(crate) fn set_usage_observer(&self, observer: Arc<agent_llm::UsageObserver>) {
        *self.usage.lock().unwrap_or_else(|e| e.into_inner()) = Some(observer);
    }

    /// Composition of the latest request — persisted beside its usage so a
    /// reopened session's context card keeps the split.
    pub(crate) fn last_breakdown(&self) -> Option<agent_store::ContextBreakdown> {
        *self.breakdown.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn usage_cache_write(&self) -> Option<u32> {
        self.usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|observer| observer.cache_write())
    }

    pub(crate) fn start(self: &Arc<Self>) {
        let relay = self.relay.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some((mut events, forward)) = relay else {
            return;
        };
        let checkpoint = Arc::downgrade(self);
        tokio::spawn(async move {
            let _cleanup = RelayCleanup {
                checkpoint: checkpoint.clone(),
                forward: forward.clone(),
            };
            while let Some(event) = events.recv().await {
                let mut cache_write = None;
                let mut breakdown = None;
                if let Some(checkpoint) = checkpoint.upgrade() {
                    if let agent_ipc::UiEvent::Custom { kind, payload } = &event {
                        if kind == BARRIER {
                            if let Some(id) = payload.as_u64() {
                                if let Some(done) = checkpoint
                                    .barriers
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .remove(&id)
                                {
                                    let _ = done.send(());
                                }
                            }
                            continue;
                        }
                    }
                    checkpoint.observe(&event);
                    // Once a tool/approval is visible, its preceding draft
                    // and any completed result must already survive a crash.
                    // Await the blocking worker here without blocking Tokio;
                    // token deltas still flow without a disk wait.
                    if matches!(
                        &event,
                        agent_ipc::UiEvent::ApprovalRequested { .. }
                            | agent_ipc::UiEvent::ToolCallStarted {
                                parent: None,
                                child_id: None,
                                ..
                            }
                            | agent_ipc::UiEvent::ToolCallFinished {
                                parent: None,
                                child_id: None,
                                ..
                            }
                    ) {
                        checkpoint.flush().await;
                    }
                    if matches!(&event, agent_ipc::UiEvent::Usage { .. }) {
                        cache_write = checkpoint.usage_cache_write();
                        breakdown = checkpoint.last_breakdown();
                    }
                }
                let _ = forward.send(event);
                // The shared Usage shape has no cache-write slot; the UI gets
                // it as a product event immediately after the Usage it belongs
                // to. Persistence reads the same observer at turn end.
                if let Some(tokens) = cache_write {
                    let _ = forward.send(agent_ipc::UiEvent::Custom {
                        kind: USAGE_CACHE_WRITE.into(),
                        payload: tokens.into(),
                    });
                }
                // Same pattern for the request's estimated composition.
                if let Some(breakdown) = breakdown {
                    let _ = forward.send(agent_ipc::UiEvent::Custom {
                        kind: CONTEXT_BREAKDOWN.into(),
                        payload: serde_json::to_value(breakdown).unwrap_or_default(),
                    });
                }
            }
        });
    }

    pub(crate) async fn drain(self: &Arc<Self>) {
        // Keep this invariant at the wait site as well as the actor entry
        // points: an unstarted relay must never leave a barrier waiting.
        self.start();
        let id = self
            .next_barrier
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (done, ready) = oneshot::channel();
        self.barriers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, done);
        if self
            .ui
            .send(agent_ipc::UiEvent::Custom {
                kind: BARRIER.into(),
                payload: id.into(),
            })
            .is_err()
        {
            self.barriers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return;
        }
        let _ = ready.await;
        self.flush().await;
    }

    /// Wait only at provider/turn boundaries, never while polling its stream.
    async fn flush(&self) {
        loop {
            let done = self.written.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if !self.writing.load(Ordering::Acquire) && !self.dirty.load(Ordering::Acquire) {
                return;
            }
            done.await;
        }
    }

    fn observe(self: &Arc<Self>, event: &agent_ipc::UiEvent) {
        let agent_ipc::UiEvent::ToolCallFinished {
            name,
            ok,
            content,
            parent: None,
            child_id: None,
            ..
        } = event
        else {
            return;
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // The hook recorded the exact call id before dispatch. The relay is
        // ordered, including veto/permission failures; nested cards never
        // consume a top-level dispatch. Do not guess by tool name.
        let Some(call) = state.dispatched.pop_front() else {
            tracing::warn!(tool = %name, "tool result has no recorded dispatch; checkpoint skipped it");
            return;
        };
        let Some(active) = &mut state.active else {
            return;
        };
        if active
            .history
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some(call.id.as_str()))
        {
            return;
        }
        if !active
            .history
            .iter()
            .flat_map(|m| m.tool_calls.iter().flatten())
            .any(|c| c.id == call.id)
        {
            return;
        }
        let mut output = ChatMessage::tool_result(call.id, content.clone());
        output.is_error = Some(!ok);
        active.history.push(output);
        self.save(state);
    }

    pub(crate) fn revision(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revision
    }

    pub(crate) fn set_revision(&self, revision: u64) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revision = revision;
    }

    pub(crate) fn begin(self: &Arc<Self>, history: &[ChatMessage], input: &str, meta: SessionMeta) {
        let mut history = history.to_vec();
        history.push(ChatMessage::user(input));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failed = false;
        state.failures = 0;
        state.notified = false;
        state.dispatched.clear();
        state.active = Some(ActiveTurn {
            request: Vec::new(),
            history,
            meta,
        });
        self.save(state);
    }

    pub(crate) fn end(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).active = None;
    }

    /// Keep the plugin chain shared for reloads, but make the recovery hook
    /// private to this engine so another session cannot consume its call ids.
    pub(crate) fn hooks(self: &Arc<Self>, shared: HookChain) -> HookChain {
        HookChain::layered(
            vec![Arc::new(RecoveryHook {
                checkpoint: Arc::downgrade(self),
            })],
            &shared,
        )
    }

    fn dispatch(&self, call: &ToolCall) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let known = state.active.as_ref().is_some_and(|active| {
            active
                .history
                .iter()
                .flat_map(|m| m.tool_calls.iter().flatten())
                .any(|c| c.id == call.id)
        });
        if known {
            state.dispatched.push_back(call.clone());
        }
    }

    pub(crate) fn wrap(self: &Arc<Self>, provider: Arc<dyn LlmProvider>) -> Arc<dyn LlmProvider> {
        Arc::new(CheckpointProvider {
            provider,
            checkpoint: self.clone(),
        })
    }

    fn request(self: &Arc<Self>, messages: &[ChatMessage], native_tools: bool) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let active = state.active.as_mut()?;
        // The kernel preserves the complete system message across rounds and
        // compaction. A different system context is a different request; never
        // recognize it by a shared first line or a prompt-text prefix.
        if messages.first() != active.history.first() {
            return None;
        }
        let mut request = messages.to_vec();
        if !native_tools {
            // Flattening preserves the following tool-result ids. Restore a
            // structured call only from those ids, with an exact wire-copy
            // check; repeated/truncated text cannot select another call.
            restore_tool_calls(&mut request, &active.history)?;
        }
        // A retry resends the same context. Preserve the old partial until the
        // replacement attempt actually produces a chunk (establishment may fail).
        if active.request != request {
            active.history = request.clone();
            active.request = request;
        }
        state.generation += 1;
        let generation = state.generation;
        self.save(state);
        Some(generation)
    }

    fn draft(self: &Arc<Self>, generation: u64, message: ChatMessage) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation != generation {
            return;
        }
        let Some(active) = &mut state.active else {
            return;
        };
        active.history.clone_from(&active.request);
        if message.content.as_ref().is_some_and(|s| !s.is_empty())
            || message.reasoning.as_ref().is_some_and(|s| !s.is_empty())
            || message.tool_calls.is_some()
        {
            active.history.push(message);
        }
        self.save(state);
    }

    /// The stream only marks the latest snapshot dirty. A single blocking
    /// worker performs file locking and serialization off Tokio workers.
    fn save(self: &Arc<Self>, state: std::sync::MutexGuard<'_, State>) {
        drop(state);
        self.dirty.store(true, Ordering::Release);
        self.start_writer();
    }

    fn start_writer(self: &Arc<Self>) {
        if self
            .writing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let checkpoint = self.clone();
        tokio::task::spawn_blocking(move || {
            let _done = WriterCleanup(checkpoint.clone());
            while checkpoint.dirty.swap(false, Ordering::AcqRel) {
                if checkpoint.save_snapshot() {
                    // Retry even when the model is idle and produces no new
                    // chunks. This sleeps only on the blocking pool.
                    std::thread::sleep(CHECKPOINT_RETRY_DELAY);
                    checkpoint.dirty.store(true, Ordering::Release);
                }
            }
        });
    }

    /// Returns true for a bounded transient retry.
    fn save_snapshot(&self) -> bool {
        let (history, recovery_rows, meta, expected) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.failed {
                return false;
            }
            let state = &mut *state;
            let Some(active) = &mut state.active else {
                return false;
            };
            active.meta.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(settings) = &state.settings {
                active.meta.permission_mode = settings
                    .permissions
                    .read()
                    .ok()
                    .map(|gate| gate.mode().as_str().to_string());
                active.meta.agent_mode = settings.agent_mode.read().ok().map(|mode| mode.clone());
                active.meta.thinking_level = settings
                    .thinking
                    .read()
                    .ok()
                    .and_then(|level| level.clone());
            }
            let (history, recovery_rows) = recovery_history(&active.history);
            (history, recovery_rows, active.meta.clone(), state.revision)
        };
        let result =
            self.store
                .checkpoint_turn_checked_with_rows(&history, meta, expected, &recovery_rows);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(revision) => {
                state.failures = 0;
                state.notified = false;
                // `set_revision` may have moved on while we wrote.
                if state.revision == expected {
                    state.revision = revision;
                }
                false
            }
            Err(error) => {
                // A stale window must never overwrite a competing writer.
                state.failures = state.failures.saturating_add(1);
                if agent_store::is_stale_revision(&error) || state.failures >= MAX_SAVE_FAILURES {
                    state.failed = true;
                }
                if !state.notified {
                    state.notified = true;
                    let _ = self.ui.send(agent_ipc::UiEvent::SystemMessage(format!(
                        "保存运行中的会话失败: {error}"
                    )));
                }
                !state.failed
            }
        }
    }
}

/// Reset the worker flag even if a serializer or file operation panics.
struct WriterCleanup(Arc<TurnCheckpoint>);

impl Drop for WriterCleanup {
    fn drop(&mut self) {
        self.0.writing.store(false, Ordering::Release);
        self.0.written.notify_waiters();
        if self.0.dirty.load(Ordering::Acquire) && !std::thread::panicking() {
            self.0.start_writer();
        }
    }
}

/// Records each top-level call id before user hooks run: vetoes and
/// permission denials also emit a result. Calls execute in order; nested
/// tools bypass this hook. It is a private layer of this engine's chain, so
/// every shared plugin hook keeps its own timeout.
struct RecoveryHook {
    checkpoint: std::sync::Weak<TurnCheckpoint>,
}

#[async_trait::async_trait]
impl AgentHook for RecoveryHook {
    fn id(&self) -> &str {
        "husk.recovery"
    }

    async fn before_tool_execute(&self, call: &mut ToolCall) -> anyhow::Result<()> {
        if let Some(checkpoint) = self.checkpoint.upgrade() {
            checkpoint.dispatch(call);
        }
        Ok(())
    }
}

/// Restore a flattened assistant only from the exact ids on its following
/// result rows. Unknown/ambiguous wire changes leave the previous recovery
/// point intact until the engine supplies its authoritative final history.
fn restore_tool_calls(request: &mut [ChatMessage], known: &[ChatMessage]) -> Option<()> {
    for index in 0..request.len() {
        if request[index].role != Role::Assistant || request[index].tool_calls.is_some() {
            continue;
        }
        let ids: Vec<_> = request[index + 1..]
            .iter()
            .take_while(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();
        if ids.is_empty() {
            continue;
        }
        let mut candidates = known.iter().filter(|message| {
            message.tool_calls.as_ref().is_some_and(|calls| {
                calls.len() == ids.len() && calls.iter().all(|call| ids.contains(&call.id.as_str()))
            })
        });
        let original = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        let mut flat = vec![original.clone()];
        agent_kernel::compaction::flatten_tool_calls_for_wire(&mut flat);
        if flat[0] != request[index] {
            return None;
        }
        request[index].content = original.content.clone();
        request[index].tool_calls = original.tool_calls.clone();
    }
    Some(())
}

impl Drop for TurnCheckpoint {
    fn drop(&mut self) {
        self.ui.close();
    }
}

/// Build the snapshot the resumed engine needs: unknown tool outcomes are
/// paired explicitly and the turn is marked interrupted. Also returns the
/// indices of those synthetic rows so display pages can hide them — the live
/// turn has not been interrupted and must not tell the user it was.
fn recovery_history(history: &[ChatMessage]) -> (Vec<ChatMessage>, Vec<usize>) {
    let outputs: std::collections::HashSet<_> = history
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.tool_call_id.as_deref())
        .collect();
    let mut recovered = Vec::with_capacity(history.len());
    let mut recovery_rows = Vec::new();
    for message in history {
        let mut message = message.clone();
        if message.role == Role::Assistant {
            if let Some(text) = &mut message.content {
                *text = crate::session::strip_call_echo(text);
                if text.is_empty() {
                    message.content = None;
                }
            }
        }
        // Pending calls have an explicit UNKNOWN outcome in the recovery copy.
        // They remain visible and strict providers receive paired calls/results.
        // The live engine never sees these placeholders or re-executes them.
        let pending: Vec<_> = message
            .tool_calls
            .iter()
            .flatten()
            .filter(|call| !outputs.contains(call.id.as_str()))
            .map(|call| ChatMessage::tool_result_err(call.id.clone(), INTERRUPTED_TOOL))
            .collect();
        recovered.push(message);
        for placeholder in pending {
            recovered.push(placeholder);
            recovery_rows.push(recovered.len() - 1);
        }
    }
    recovered.push(ChatMessage::notice(INTERRUPTED_TURN));
    recovery_rows.push(recovered.len() - 1);
    (recovered, recovery_rows)
}

struct CheckpointProvider {
    provider: Arc<dyn LlmProvider>,
    checkpoint: Arc<TurnCheckpoint>,
}

#[async_trait::async_trait]
impl LlmProvider for CheckpointProvider {
    fn id(&self) -> &'static str {
        self.provider.id()
    }

    fn capabilities(&self) -> Capabilities {
        self.provider.capabilities()
    }

    fn native_tool_calls(&self) -> bool {
        self.provider.native_tool_calls()
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
        self.checkpoint.drain().await;
        *self
            .checkpoint
            .breakdown
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(crate::context_breakdown::estimate(
            messages,
            tools.as_ref(),
            crate::session::is_memory_context,
        ));
        let generation = self.checkpoint.request(messages, self.native_tool_calls());
        self.checkpoint.flush().await;
        let stream = self
            .provider
            .chat_stream(
                model,
                messages,
                tools,
                temperature,
                reasoning_effort,
                params,
            )
            .await?;
        let Some(generation) = generation else {
            return Ok(stream);
        };
        Ok(Box::pin(CheckpointStream {
            inner: stream,
            checkpoint: self.checkpoint.clone(),
            generation,
            message: ChatMessage::assistant(""),
            assembler: ToolCallAssembler::new(),
            reasoning_started: None,
            dirty: false,
            first_chunk: true,
            timer: tokio::time::interval(CHECKPOINT_INTERVAL),
        }))
    }
}

struct CheckpointStream {
    inner: BoxStream<StreamChunk>,
    checkpoint: Arc<TurnCheckpoint>,
    generation: u64,
    message: ChatMessage,
    assembler: ToolCallAssembler,
    reasoning_started: Option<i64>,
    dirty: bool,
    first_chunk: bool,
    timer: tokio::time::Interval,
}

impl CheckpointStream {
    fn flush(&mut self) {
        if self.dirty {
            self.checkpoint.draft(self.generation, self.message.clone());
            self.dirty = false;
        }
    }
}

impl Stream for CheckpointStream {
    type Item = anyhow::Result<StreamChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.timer.poll_tick(cx).is_ready() {
            self.flush();
        }
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                match &chunk {
                    StreamChunk::ContentDelta(text) => {
                        self.message
                            .content
                            .get_or_insert_with(String::new)
                            .push_str(text);
                        self.dirty = true;
                    }
                    StreamChunk::ReasoningDelta(text) => {
                        let now = now_ms();
                        let started = *self.reasoning_started.get_or_insert(now);
                        self.message
                            .reasoning
                            .get_or_insert_with(String::new)
                            .push_str(text);
                        self.message.duration_ms = Some(now - started);
                        self.dirty = true;
                    }
                    StreamChunk::ToolCallDelta { .. } => {
                        self.assembler.feed(&chunk);
                    }
                    StreamChunk::Done { .. } => {
                        let calls: Vec<_> = std::mem::take(&mut self.assembler)
                            .finish()
                            .into_iter()
                            .filter(|c| !c.name.trim().is_empty())
                            .collect();
                        self.message.tool_calls = (!calls.is_empty()).then_some(calls);
                        self.dirty = true;
                        self.flush();
                    }
                    StreamChunk::Error(_) => self.flush(),
                }
                if self.first_chunk {
                    self.first_chunk = false;
                    self.dirty = true;
                    self.flush();
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.flush();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.flush();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for CheckpointStream {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
