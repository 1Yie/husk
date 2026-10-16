//! Durable recovery points for a turn that has not returned to its actor yet.
//!
//! The engine owns a mutable history for the entire turn. Its provider boundary
//! exposes each round's canonical context, while the returned stream exposes the
//! draft that has not entered that history yet. Keep both on disk; never depend
//! on Drop or a cooperative cancellation to save a running task.

use agent_kernel::channels::{UiReceiver, UiSink};
use agent_llm::types::{now_ms, ChatMessage, Role, StreamChunk, ToolCallAssembler};
use agent_llm::{BoxStream, Capabilities, LlmProvider, ModelParams};
use agent_store::{SessionMeta, SessionStore};
use futures::Stream;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::oneshot;

const BARRIER: &str = "husk:checkpoint_barrier";

const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(250);
const INTERRUPTED_TURN: &str = "上次任务因会话意外中断而停止，已恢复到最后保存的进度。";
const INTERRUPTED_TOOL: &str = "会话在工具结果保存前中断，执行结果未知。继续前请检查实际状态，不要假定操作未执行，也不要直接重复有副作用的操作。";

pub(crate) struct TurnCheckpoint {
    store: Arc<SessionStore>,
    ui: UiSink,
    state: Mutex<State>,
    /// Serializes disk writes so `state` is never held across IO.
    writer: Mutex<()>,
    relay: Mutex<Option<(UiReceiver, UiSink)>>,
    barriers: Mutex<std::collections::HashMap<u64, oneshot::Sender<()>>>,
    next_barrier: std::sync::atomic::AtomicU64,
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
            writer: Mutex::new(()),
            state: Mutex::new(State {
                revision,
                generation: 0,
                active: None,
                failed: false,
                failures: 0,
                notified: false,
                settings: None,
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
                }
                let _ = forward.send(event);
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
    }

    fn observe(&self, event: &agent_ipc::UiEvent) {
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
        let Some(active) = &mut state.active else {
            return;
        };
        // The engine dispatches top-level calls sequentially. Parallel batch
        // and delegated events carry a parent/child marker and are excluded
        // above. The provider barrier drains this queue before the next
        // request replaces the round, so this is the current round's FIFO.
        let outputs: std::collections::HashSet<_> = active
            .history
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();
        let id = active
            .history
            .iter()
            .flat_map(|m| m.tool_calls.iter().flatten())
            .find(|call| call.name == *name && !outputs.contains(call.id.as_str()))
            .map(|call| call.id.clone());
        if let Some(id) = id {
            let mut output = ChatMessage::tool_result(id, content.clone());
            output.is_error = Some(!ok);
            active.history.push(output);
            self.save(state);
        } else {
            tracing::warn!(tool = %name, "tool result matched no pending call; checkpoint skipped it");
        }
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

    pub(crate) fn begin(&self, history: &[ChatMessage], input: &str, meta: SessionMeta) {
        let mut history = history.to_vec();
        history.push(ChatMessage::user(input));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failed = false;
        state.failures = 0;
        state.notified = false;
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

    pub(crate) fn wrap(self: &Arc<Self>, provider: Arc<dyn LlmProvider>) -> Arc<dyn LlmProvider> {
        Arc::new(CheckpointProvider {
            provider,
            checkpoint: self.clone(),
        })
    }

    fn request(&self, messages: &[ChatMessage], native_tools: bool) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let active = state.active.as_mut()?;
        // The same sampler also serves compaction. Those requests have their
        // own system prompt and must never replace the user's conversation.
        let system_line = |m: &ChatMessage| {
            m.content
                .as_deref()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_owned()
        };
        if messages.first().map(system_line) != active.history.first().map(system_line) {
            return None;
        }
        let mut request = messages.to_vec();
        if !native_tools {
            // The kernel flattens calls only on its wire copy. Restore known
            // structured rows so saving that copy cannot erase tool history.
            let mut flat = active.history.clone();
            agent_kernel::compaction::flatten_tool_calls_for_wire(&mut flat);
            for message in &mut request {
                if let Some((original, _)) =
                    active.history.iter().zip(&flat).find(|(original, wire)| {
                        original.tool_calls.is_some()
                            && wire.role == message.role
                            && wire.content == message.content
                    })
                {
                    message.content = original.content.clone();
                    message.tool_calls = original.tool_calls.clone();
                }
            }
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

    fn draft(&self, generation: u64, message: ChatMessage) {
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

    /// Persist the newest recovery point without holding `state` across IO.
    /// Callers hand over their guard after mutating; saves are serialized by
    /// `writer`, and each one re-snapshots, so concurrent updates coalesce.
    fn save(&self, state: std::sync::MutexGuard<'_, State>) {
        drop(state);
        let _writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let (history, meta, expected) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.failed {
                return;
            }
            let state = &mut *state;
            let Some(active) = &mut state.active else {
                return;
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
            (
                recovery_history(&active.history),
                active.meta.clone(),
                state.revision,
            )
        };
        let result = self.store.commit_turn_checked(&history, meta, expected);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(revision) => {
                state.failures = 0;
                state.notified = false;
                // `set_revision` may have moved on while we wrote.
                if state.revision == expected {
                    state.revision = revision;
                }
            }
            Err(error) => {
                // A stale window must never retry by overwriting another
                // writer. Other IO errors (full disk, a busy lock) are retried
                // by the next save, a bounded number of times.
                state.failures = state.failures.saturating_add(1);
                if agent_store::is_stale_revision(&error) || state.failures >= MAX_SAVE_FAILURES {
                    state.failed = true;
                }
                if !state.notified || state.failed {
                    state.notified = true;
                    let _ = self.ui.send(agent_ipc::UiEvent::SystemMessage(format!(
                        "保存运行中的会话失败: {error}"
                    )));
                }
            }
        }
    }
}

impl Drop for TurnCheckpoint {
    fn drop(&mut self) {
        self.ui.close();
    }
}

fn recovery_history(history: &[ChatMessage]) -> Vec<ChatMessage> {
    let outputs: std::collections::HashSet<_> = history
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.tool_call_id.as_deref())
        .collect();
    let mut recovered = Vec::with_capacity(history.len());
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
        recovered.extend(pending);
    }
    recovered.push(ChatMessage::notice(INTERRUPTED_TURN));
    recovered
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
        let generation = self.checkpoint.request(messages, self.native_tool_calls());
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
