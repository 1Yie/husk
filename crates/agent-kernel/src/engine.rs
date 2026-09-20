//! `AgentState` machine + the ReAct turn loop.
//!
//! Transitions (kernel-architecture.md §AgentState):
//!
//! ```text
//! Idle→ScanningWorkspace→Reasoning→StreamingToken
//!     →{Finished | AwaitingToolConfirmation→ExecutingTool→Reasoning}
//! ```
//!
//! `Failed` is terminal per turn, not per session.
//!
//! Stage 4 runs this headless on `MockProvider` — the loop is: sample →
//! collect text + tool calls → execute each tool → append results as a
//! `tool` message → re-sample, until a turn ends with no pending tool calls.
//! Permission gating (Stage 6) inserts `AwaitingToolConfirmation` between
//! "tool call arrived" and "tool executes"; the loop already emits the state
//! transition so the wiring point is explicit.

use std::collections::VecDeque;
use std::sync::Arc;

use agent_ipc::{AgentState, UiEvent};
use agent_llm::sampler::{SampleRequest, Sampler};
use agent_llm::types::{ChatMessage, StreamChunk, ToolCallAssembler};
use tokio::sync::mpsc;
use tracing::instrument;

use crate::compaction::{self, CompactionSuppressor};
use crate::permissions::{Decision, PermissionGate};
use crate::tools::{ToolCtx, ToolRegistry};

/// The engine's observable side — channels + sink for UI events.
pub struct EngineIo {
    pub ui_tx: mpsc::Sender<UiEvent>,
    /// Mid-turn steering texts — SessionActor forwards `UiCommand::Steer`
    /// into this channel; the engine drains it between tool calls.
    pub steer_rx: mpsc::Receiver<String>,
    /// Cooperative cancel flag — the UI/actor sets it (bypassing the command
    /// pump so it lands mid-turn); the engine checks it between chunks and
    /// before each tool dispatch. `Arc` shared with `SessionActor::cancel`.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Per-turn outcome handed back to `SessionActor`.
#[derive(Debug)]
pub struct TurnOutcome {
    /// Final assistant text (concatenated content deltas).
    pub text: String,
    /// Usage from the last `Done` chunk.
    pub usage: Option<(u32, u32)>,
    /// Tool calls executed this turn (for hunk attribution / UI list).
    pub tool_calls_run: usize,
}

pub struct Engine {
    sampler: Sampler,
    registry: Arc<ToolRegistry>,
    ctx: Arc<ToolCtx>,
    model: String,
    temperature: f32,
    /// Pending mid-turn steering text drained from `cmd_rx`.
    steer_queue: VecDeque<String>,
    /// Stage 6: the permission gate consulted before every tool dispatch.
    permissions: Arc<std::sync::RwLock<PermissionGate>>,
    /// Decision slot shared with SessionActor — the engine can't hold a
    /// receiver `&mut self`-locked across `run_turn`, so the actor writes
    /// `Some((request_id, approved))` here and the engine polls it between
    /// polls. The `request_id` correlates the verdict to a specific
    /// `ApprovalRequested` — a stale click whose id doesn't match the
    /// pending request is ignored instead of approving a different tool
    /// (P1-a).
    decision: Arc<std::sync::Mutex<Option<(u64, bool)>>>,
    /// Monotone id source for `ApprovalRequested` correlation.
    next_request_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Context window for the active model — compaction triggers at 80%.
    context_window: usize,
    /// Stage 7: compaction retry-storm suppression.
    compaction_suppressor: CompactionSuppressor,
    /// Stage 9: ordered hook chain (veto/mutate before+after tools).
    hooks: crate::hooks::HookChain,
    /// Plugin tool router — built-in names win; plugins fill the rest.
    plugin_router: Option<Arc<agent_plugin::PluginManager>>,
    /// Thinking / reasoning intensity level (e.g. "off", "low", "medium",
    /// "high", "max"). Shared slot like `permissions` — the bridge reads it
    /// to report a session's live level back to the UI.
    thinking_level: Arc<std::sync::RwLock<Option<String>>>,
    /// Model-specific thinking level mapping from pi-agent config schema.
    thinking_level_map: Option<std::collections::HashMap<String, Option<String>>>,
}

impl Engine {
    pub fn new(
        provider: Arc<dyn agent_llm::LlmProvider>,
        registry: Arc<ToolRegistry>,
        ctx: Arc<ToolCtx>,
        model: impl Into<String>,
        temperature: f32,
    ) -> Self {
        Self {
            sampler: Sampler::new(provider),
            registry,
            ctx,
            model: model.into(),
            temperature,
            steer_queue: VecDeque::new(),
            permissions: Arc::new(std::sync::RwLock::new(PermissionGate::from_mode_str("default"))),
            decision: Arc::new(std::sync::Mutex::new(None)),
            next_request_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
            context_window: 256_000,
            compaction_suppressor: CompactionSuppressor::default(),
            hooks: crate::hooks::HookChain::new(),
            plugin_router: None,
            thinking_level: Arc::new(std::sync::RwLock::new(None)),
            thinking_level_map: None,
        }
    }

    /// Set thinking / reasoning intensity level.
    pub fn set_thinking_level(&mut self, level: Option<String>) {
        if let Ok(mut w) = self.thinking_level.write() {
            *w = level;
        }
    }

    /// Current thinking level if set.
    pub fn thinking_level(&self) -> Option<String> {
        self.thinking_level.read().ok().and_then(|l| l.clone())
    }

    /// Access the shared thinking-level slot — SessionManager keeps a clone
    /// on the session handle so `model_info` can report the live value.
    pub fn thinking_level_shared(&self) -> Arc<std::sync::RwLock<Option<String>>> {
        self.thinking_level.clone()
    }

    /// Set model-specific thinking level mapping.
    pub fn set_thinking_level_map(
        &mut self,
        map: Option<std::collections::HashMap<String, Option<String>>>,
    ) {
        self.thinking_level_map = map;
    }

    /// Resolves the effective wire reasoning effort ("low", "medium", "high", "max" or None).
    pub fn resolve_reasoning_effort(&self) -> Option<String> {
        let guard = self.thinking_level.read().ok()?;
        let level = guard.as_deref()?;
        if level == "off" {
            return None;
        }
        if let Some(map) = &self.thinking_level_map {
            if let Some(val) = map.get(level) {
                return val.clone();
            }
        }
        match level {
            "minimal" => Some("minimal".into()),
            "low" => Some("low".into()),
            "medium" => Some("medium".into()),
            "high" => Some("high".into()),
            "xhigh" => Some("xhigh".into()),
            "max" => Some("max".into()),
            other => Some(other.to_string()),
        }
    }

    /// SessionActor installs the plugin manager once plugins register.
    pub fn set_plugins(&mut self, mgr: Arc<agent_plugin::PluginManager>) {
        self.plugin_router = Some(mgr);
    }

    /// SessionActor installs the hook chain (registered plugins/built-ins).
    pub fn set_hooks(&mut self, chain: crate::hooks::HookChain) {
        self.hooks = chain;
    }

    /// The active model's context window — SessionActor sets it from config.
    pub fn set_context_window(&mut self, window: usize) {
        self.context_window = window;
    }

    /// Current context-window bound — model config value or the 256k
    /// default (same bound `Usage` events report).
    pub fn context_window(&self) -> usize {
        self.context_window
    }

    /// SessionActor installs the permission gate (mode + repo rules).
    pub fn set_permissions(&mut self, gate: PermissionGate) {
        if let Ok(mut w) = self.permissions.write() {
            *w = gate;
        }
    }

    /// Access the shared permission gate handle for direct mid-turn updates.
    pub fn permissions_writer(&self) -> Arc<std::sync::RwLock<PermissionGate>> {
        self.permissions.clone()
    }

    /// The shared decision slot — SessionActor clones it to deliver
    /// `ToolDecision { request_id, approved }` while the engine is
    /// mid-`run_turn`.
    pub fn decision_slot(&self) -> Arc<std::sync::Mutex<Option<(u64, bool)>>> {
        self.decision.clone()
    }

    /// Hot-swap provider for the next turn.
    pub fn set_provider(&mut self, provider: Arc<dyn agent_llm::LlmProvider>) {
        self.sampler.set_provider(provider);
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    /// Drain any pending steering texts between tool calls — marks the
    /// turn `steered` for UI provenance.
    fn drain_steering(&mut self, io: &mut EngineIo, history: &mut Vec<ChatMessage>) {
        while let Ok(text) = io.steer_rx.try_recv() {
            self.steer_queue.push_back(text);
        }
        if !self.steer_queue.is_empty() {
            // Notify the UI the turn was steered (provenance marker).
            let line = "已插入引导指令".to_string();
            let _ = io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
            history.push(ChatMessage::notice(line));
        }
    }

    /// Run one full user turn: prompt → sample → tools → re-sample … → done.
    ///
    /// `history` is the session's conversation; the engine appends the user
    /// turn, tool calls, and tool results to it in place. `hunks` records
    /// every write tool's before/after for undo/rewind (Stage 6).
    #[instrument(skip_all, name = "turn")]
    pub async fn run_turn(
        &mut self,
        io: &mut EngineIo,
        history: &mut Vec<ChatMessage>,
        user_text: String,
        hunks: &mut agent_context::HunkTracker,
    ) -> Result<TurnOutcome, String> {
        history.push(ChatMessage::user(user_text));
        self.set_state(io, AgentState::Reasoning);

        // ---- Stage 7: sanitize + compaction check before sampling ----
        // Sanitize runs every turn (flatten tool calls, strip reasoning,
        // budget-fit); compaction only when estimate crosses 80% of window.
        // P1-b: only flatten tool_calls into text when the provider uses a
        // text protocol — a native function-calling provider needs the
        // structured array intact or every Role::Tool result orphans.
        let native_tc = self.sampler.provider().native_tool_calls();
        compaction::sanitize_for_sample(
            history,
            self.context_window.saturating_mul(9) / 10,
            native_tc,
        );
        let est = compaction::estimate_tokens(history);
        if compaction::should_compact(est, self.context_window)
            && self.compaction_suppressor.check()
        {
            self.set_state(io, AgentState::Compacting);
            match self.compact_history(io, history).await {
                Ok(()) => {
                    self.compaction_suppressor.on_success();
                    let line = format!("已压缩历史上下文（原约 {est} tokens）");
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                    history.push(ChatMessage::notice(line));
                }
                Err(e) => {
                    self.compaction_suppressor.on_failure();
                    let line = format!("上下文压缩失败: {e}");
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                    history.push(ChatMessage::notice(line));
                }
            }
            self.set_state(io, AgentState::Reasoning);
        }

        let mut text = String::new();
        let mut usage = None;
        let mut tool_calls_run = 0usize;
        let mut tool_rounds = 0usize;
        let mut force_no_tools = false;
        const MAX_TOOL_ROUNDS: usize = 128; // guard against runaway tool loops
        /// Cancel sentinel — the `Err` value and `Failed` reason (never
        /// rendered verbatim; the status bar localizes `Failed`). The
        /// user-facing line is `CANCEL_TEXT`.
        const CANCEL_ERR: &str = "turn cancelled by user";
        /// The one system line shown for a cancelled turn — a single
        /// reminder, in the UI's language.
        const CANCEL_TEXT: &str = "已被用户中断";

        loop {
            // P0-C4: cooperative cancel — checked at the top of every
            // sample/tool round so a Cancel lands between model calls even
            // if no chunk is flowing.
            if io.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                self.set_state(io, AgentState::Failed(CANCEL_ERR.into()));
                let _ = io.ui_tx.try_send(UiEvent::SystemMessage(CANCEL_TEXT.into()));
                history.push(ChatMessage::notice(CANCEL_TEXT));
                return Err(CANCEL_ERR.into());
            }

            // Inject any mid-turn steering as a user message before sampling.
            self.drain_steering(io, history);
            while let Some(steer) = self.steer_queue.pop_front() {
                // Echo the steer as a user bubble — the live stream never
                // showed it otherwise (the composer doesn't echo), while a
                // reloaded view rebuilt it from history. One render path.
                let _ = io.ui_tx.try_send(UiEvent::UserPrompt(steer.clone()));
                history.push(ChatMessage::user(format!("The user interrupted: {steer}")));
            }

            // Budget check + sanitize before each sample pass
            let native_tc = self.sampler.provider().native_tool_calls();
            compaction::sanitize_for_sample(
                history,
                self.context_window.saturating_mul(9) / 10,
                native_tc,
            );

            let mut assembler = ToolCallAssembler::new();
            let mut round_text = String::new();
            let mut saw_done = false;
            // Delta coalescing — the UI doesn't need one event per SSE
            // chunk (often 1-5 chars). Buffer text/reasoning deltas and
            // flush at ~120 chars or on any non-delta chunk so the
            // bounded UI channel never sees a per-token burst large
            // enough to push out control events via try_send.
            let mut pending_text_delta = String::new();
            let mut pending_reasoning_delta = String::new();
            // A provider-level `StreamChunk::Error` (e.g. devin's
            // `internal_server_error … upstream error`) arrives inside the
            // stream, not as a transport `Err` — without this flag the turn
            // would fall through to the no-calls branch and persist an EMPTY
            // assistant message into history, corrupting every later request.
            let mut stream_error: Option<String> = None;
            // Sampler policy notices (retry / thinking-degrade) shown live —
            // collected here and persisted after the sample so a reloaded
            // view replays them too.
            let mut policy_notes: Vec<String> = Vec::new();

            let effort = self.resolve_reasoning_effort();
            let schema = self.registry.request_schema();
            let tools_opt = if force_no_tools {
                None
            } else {
                Some(&schema)
            };
            let req = SampleRequest {
                model: &self.model,
                temperature: self.temperature,
                tools: tools_opt,
                reasoning_effort: effort.as_deref(),
            };

            // Race the sample against the cancel flag — a Cancel mid-stream
            // drops the in-flight request instead of waiting for the next
            // chunk or the 300 s idle timeout. The flag is cloned out so the
            // watcher doesn't borrow `io` while the chunk closure does.
            let cancel_flag = io.cancel.clone();
            let cancel_watch = async move {
                while !cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            };

            let res = tokio::select! {
                biased;
                _ = cancel_watch => {
                    Err(agent_llm::sampler::SampleError::Stream(CANCEL_ERR.into()))
                }
                r = self.sampler.sample(
                    req,
                    history,
                    |chunk| {
                        match chunk {
                            StreamChunk::ReasoningDelta(t) => {
                                pending_reasoning_delta.push_str(t);
                                if pending_reasoning_delta.len() >= 120 {
                                    let _ = io.ui_tx.try_send(UiEvent::ReasoningDelta(
                                        std::mem::take(&mut pending_reasoning_delta),
                                    ));
                                }
                            }
                            StreamChunk::ContentDelta(t) => {
                                round_text.push_str(t);
                                pending_text_delta.push_str(t);
                                if pending_text_delta.len() >= 120 {
                                    let _ = io.ui_tx.try_send(UiEvent::TextDelta(
                                        std::mem::take(&mut pending_text_delta),
                                    ));
                                }
                            }
                            StreamChunk::ToolCallDelta { .. } => {
                                assembler.feed(chunk);
                            }
                            StreamChunk::Done { prompt_tokens, completion_tokens } => {
                                saw_done = true;
                                // Flush any coalesced deltas BEFORE the
                                // terminal event lands so the UI never
                                // renders trailing text after Finished.
                                if !pending_text_delta.is_empty() {
                                    let _ = io.ui_tx.try_send(UiEvent::TextDelta(
                                        std::mem::take(&mut pending_text_delta),
                                    ));
                                }
                                if !pending_reasoning_delta.is_empty() {
                                    let _ = io.ui_tx.try_send(UiEvent::ReasoningDelta(
                                        std::mem::take(&mut pending_reasoning_delta),
                                    ));
                                }
                                if let (Some(p), Some(c)) = (prompt_tokens, completion_tokens) {
                                    usage = Some((*p, *c));
                                    let _ = io.ui_tx.try_send(UiEvent::Usage {
                                        prompt_tokens: *p,
                                        completion_tokens: *c,
                                        context_window: self.context_window as u32,
                                    });
                                }
                            }
                            StreamChunk::Error(e) => {
                                stream_error = Some(e.clone());
                                if !pending_text_delta.is_empty() {
                                    let _ = io.ui_tx.try_send(UiEvent::TextDelta(
                                        std::mem::take(&mut pending_text_delta),
                                    ));
                                }
                                if !pending_reasoning_delta.is_empty() {
                                    let _ = io.ui_tx.try_send(UiEvent::ReasoningDelta(
                                        std::mem::take(&mut pending_reasoning_delta),
                                    ));
                                }
                                // The `*[error: e]*` marker + turn-end `Error` event
                                // report this — no extra SystemMessage (it printed
                                // the same text a second time live).
                            }
                        }
                    },
                    |ev| {
                        let t = format!("{ev:?}");
                        let _ = io.ui_tx.try_send(UiEvent::SystemMessage(t.clone()));
                        policy_notes.push(t);
                    },
                ) => r,
            };

            // Flush any sub-threshold deltas left after the sample
            // returned — a stream that ended cleanly before hitting 120
            // chars would otherwise strand its tail in the buffer.
            if !pending_text_delta.is_empty() {
                let _ = io.ui_tx.try_send(UiEvent::TextDelta(
                    std::mem::take(&mut pending_text_delta),
                ));
            }
            if !pending_reasoning_delta.is_empty() {
                let _ = io.ui_tx.try_send(UiEvent::ReasoningDelta(
                    std::mem::take(&mut pending_reasoning_delta),
                ));
            }

            // Persist the sampler policy notices — they replayed live,
            // so the snapshot must carry them (cancel/error paths return
            // early below; drain before them).
            for t in policy_notes.drain(..) {
                history.push(ChatMessage::notice(t));
            }

            // A cancel mid-sample unwinds here — the partial text and the
            // cancel line persist separately so a reloaded view replays
            // exactly what the live stream showed (text, then the notice).
            if io.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                if !round_text.is_empty() {
                    history.push(ChatMessage::assistant(round_text.clone()));
                }
                history.push(ChatMessage::notice(CANCEL_TEXT));
                self.set_state(io, AgentState::Failed(CANCEL_ERR.into()));
                let _ = io.ui_tx.try_send(UiEvent::SystemMessage(CANCEL_TEXT.into()));
                return Err(CANCEL_ERR.into());
            }

            if let Err(e) = res {
                // ---- Stream salvage (hardening §4) ----
                // Partial text + the interrupt line + the error line all
                // persist — replay matches the live stream line-for-line.
                if !round_text.is_empty() {
                    history.push(ChatMessage::assistant(round_text.clone()));
                    let line = "流传输中断 — 已保留部分内容";
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(line.into()));
                    history.push(ChatMessage::notice(line));
                }
                self.set_state(io, AgentState::Failed(e.to_string()));
                let _ = io.ui_tx.try_send(UiEvent::Error(e.to_string()));
                history.push(ChatMessage::notice_error(e.to_string()));
                return Err(e.to_string());
            }
            // Provider surfaced an in-stream error (upstream 5xx, rate limit,
            // model refusal, …) — treat the turn as failed. Salvage the
            // partial text and persist the error line (the `⚠` the live
            // stream drew) so a reloaded view loses nothing.
            if let Some(e) = stream_error {
                if !round_text.is_empty() {
                    history.push(ChatMessage::assistant(round_text.clone()));
                }
                history.push(ChatMessage::notice_error(e.clone()));
                self.set_state(io, AgentState::Failed(e.clone()));
                let _ = io.ui_tx.try_send(UiEvent::Error(e.clone()));
                return Err(e);
            }
            debug_assert!(saw_done, "sampler guarantees Done exactly once");

            text.push_str(&round_text);
            let calls = assembler.finish();

            if calls.is_empty() {
                // No tool calls → turn complete.
                let final_text = if round_text.trim().is_empty() && !text.trim().is_empty() {
                    text.clone()
                } else {
                    round_text
                };

                if final_text.trim().is_empty() && tool_calls_run > 0 && !force_no_tools {
                    force_no_tools = true;
                    history.push(ChatMessage::user_hidden(
                        "Please provide your final answer and summary to the user based on the tool results above.",
                    ));
                    continue;
                }

                history.push(ChatMessage {
                    role: agent_llm::Role::Assistant,
                    content: if final_text.is_empty() { None } else { Some(final_text.clone()) },
                    tool_calls: None,
                    tool_call_id: None,
                    is_error: None,
                        notice: None,
                    ts: Some(agent_llm::types::now_ms()),
                });
                self.set_state(io, AgentState::Finished);
                let _ = io.ui_tx.try_send(UiEvent::AssistantMessage(final_text));
                return Ok(TurnOutcome { text, usage, tool_calls_run });
            }

            // Append the assistant turn with its tool calls.
            history.push(ChatMessage {
                role: agent_llm::Role::Assistant,
                content: if round_text.is_empty() { None } else { Some(round_text.clone()) },
                tool_calls: Some(calls.clone()),
                tool_call_id: None,
                is_error: None,
                        notice: None,
                ts: Some(agent_llm::types::now_ms()),
            });

            // Execute each tool call.
            tool_rounds += 1;

            for (call_idx, call) in calls.iter().enumerate() {
                // P0-C4: cancel between tool dispatches — a destructive tool
                // must not fire after the user already hit Cancel.
                if io.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    // Fill the undispatched calls — an assistant row with
                    // tool_calls but missing tool results orphans the next
                    // request (strict providers reject it) and makes the
                    // reloaded view silently drop chips the live view had.
                    for pending_call in calls.iter().skip(call_idx) {
                        history.push(ChatMessage::tool_result_err(
                            pending_call.id.clone(),
                            "cancelled before dispatch",
                        ));
                    }
                    history.push(ChatMessage::notice(CANCEL_TEXT));
                    self.set_state(io, AgentState::Failed(CANCEL_ERR.into()));
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(CANCEL_TEXT.into()));
                    return Err(CANCEL_ERR.into());
                }
                // Skip degenerate calls — a text-protocol echo can produce an
                // empty-name or empty-args call that must never dispatch
                // (it'd surface as `unknown tool ''` and poison history).
                if call.name.trim().is_empty() {
                    let line = "已跳过一个格式异常的空工具调用".to_string();
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                    history.push(ChatMessage::notice(line));
                    continue;
                }
                tool_calls_run += 1;

                let args: serde_json::Value =
                    serde_json::from_str(&call.arguments).unwrap_or_else(|_| {
                        serde_json::json!({ "_malformed": call.arguments })
                    });

                // Primary locator (path/command/pattern) for the tool call —
                // preserved in full so audit/confirmation tooltips can display
                // the complete command or path without losing critical arguments.
                let args_preview = args
                    .get("path")
                    .or_else(|| args.get("command"))
                    .or_else(|| args.get("pattern"))
                    .or_else(|| args.get("query"))
                    .or_else(|| args.get("file"))
                    .or_else(|| args.get("action"))
                    .or_else(|| args.get("url"))
                    .or_else(|| args.get("target"))
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        let s = s.trim();
                        if s.chars().count() > 4096 {
                            let mut chars = s.chars();
                            let prefix: String = chars.by_ref().take(4096).collect();
                            format!("{prefix}...")
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_else(|| {
                        if let Some(obj) = args.as_object() {
                            for v in obj.values() {
                                if let Some(s) = v.as_str() {
                                    let s = s.trim();
                                    if !s.is_empty() {
                                        if s.chars().count() > 4096 {
                                            let mut chars = s.chars();
                                            let prefix: String = chars.by_ref().take(4096).collect();
                                            return format!("{prefix}...");
                                        } else {
                                            return s.to_string();
                                        }
                                    }
                                }
                            }
                        }
                        String::new()
                    });
                let _ = io.ui_tx.try_send(UiEvent::ToolCallStarted {
                    name: call.name.clone(),
                    args_preview,
                });
                self.set_state(io, AgentState::ExecutingTool {
                    tool_name: call.name.clone(),
                });

                // ---- Stage 9: before_tool hooks (veto/mutate, pre-gate) ----
                let mut call_mut = call.clone();
                if !self.hooks.run_before_tool(&mut call_mut).await {
                    let msg = format!("tool `{}` vetoed by hook", call.name);
                    let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                        name: call.name.clone(), ok: false,
                        content: msg.clone(), ui_type: None,
                    });
                    history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                    continue;
                }
                let call = &call_mut; // hooks may have rewritten args

                // ---- Stage 6: permission gate before dispatch ----
                let is_readonly = self.registry.is_readonly(&call.name);
                let shell_cmd = args.get("command").and_then(|v| v.as_str());
                let diff_summary = args.get("path").and_then(|v| v.as_str())
                    .map(|p| format!("{p}")).unwrap_or_else(|| call.name.clone());

                // P1-c: write tools (fuzzy_patch/apply_patch/write_file) are
                // dispatched EARLY — they return a `PendingWrite` instead of
                // writing inside the tool, so we can show the REAL diff +
                // fuzzy flag on the approval card and only commit the write
                // after the user approves. Read-only/bash tools keep the
                // classic gate→dispatch flow (they have side effects we
                // can't dry-run).
                let is_write = matches!(call.name.as_str(),
                    "fuzzy_patch" | "apply_patch" | "write_file");

                // Dry-run a write tool to harvest diff/fuzzy/pending_write.
                // `dispatch` on these tools is side-effect-free (returns
                // PendingWrite), so it's safe to run pre-approval.
                let staged: Option<crate::tools::registry::ToolResult> = if is_write {
                    match self.registry.dispatch(&call.name, args.clone(), self.ctx.clone()).await {
                        Ok(res) => Some(res),
                        Err(e) => {
                            let msg = e.to_string();
                            let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                                name: call.name.clone(), ok: false,
                                content: msg.clone(), ui_type: None,
                            });
                            history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                            continue;
                        }
                    }
                } else {
                    None
                };
                let real_diff = staged.as_ref()
                    .map(|r| r.content.clone())
                    .unwrap_or_else(|| args.get("replace").and_then(|v| v.as_str()).unwrap_or("").to_string());
                let real_fuzzy = staged.as_ref().map(|r| r.fuzzy).unwrap_or(false);

                let decision = self
                    .permissions
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .decide(&call.name, is_readonly, shell_cmd, diff_summary);

                match decision {
                    Decision::Deny { reason } => {
                        let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                            name: call.name.clone(), ok: false,
                            content: reason.clone(), ui_type: None,
                        });
                        history.push(ChatMessage::tool_result_err(call.id.clone(), reason));
                        continue; // skip dispatch — denied by policy
                    }
                    Decision::Ask { diff_summary } => {
                        // Pause in AwaitingToolConfirmation; the UI's
                        // ToolDecision resolves it. Wait on the shared slot
                        // (SessionActor writes it via decision_slot()).
                        // P1-a: a fresh request_id correlates this specific
                        // pending approval to its verdict — a stale decision
                        // from an earlier card can't approve this tool.
                        let request_id = self.next_request_id
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        self.set_state(io, AgentState::AwaitingToolConfirmation {
                            tool_name: call.name.clone(),
                            diff_summary: diff_summary.clone(),
                        });
                        // P1-c: show the REAL unified diff + fuzzy flag the
                        // staged run produced — the card no longer lies
                        // about what will be written.
                        let _ = io.ui_tx.try_send(UiEvent::ApprovalRequested {
                            request_id,
                            tool_name: call.name.clone(),
                            diff: real_diff.clone(),
                            fuzzy: real_fuzzy,
                        });
                        let approved = self.wait_for_decision(request_id, &io.cancel).await;
                        self.set_state(io, AgentState::ExecutingTool {
                            tool_name: call.name.clone(),
                        });
                        if !approved {
                            let msg = format!("user denied {call_name}", call_name = call.name);
                            let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                                name: call.name.clone(), ok: false,
                                content: msg.clone(), ui_type: None,
                            });
                            history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                            continue;
                        }
                    }
                    Decision::Allow => {}
                }



                // Resolve the tool result. For a staged write tool we already
                // have it; otherwise dispatch now (read-only/bash).
                let (mut content, ui_type, mut ok, pending_write) = if let Some(res) = staged {
                    let ui_type = res.ui_type.map(|s| s.to_string());
                    (res.content, ui_type, true, res.pending_write)
                } else {
                    match self
                        .registry
                        .dispatch(&call.name, args.clone(), self.ctx.clone())
                        .await
                    {
                        Ok(res) => (
                            res.content,
                            res.ui_type.map(|s| s.to_string()),
                            true,
                            res.pending_write,
                        ),
                        Err(e) => {
                            // Built-in miss → try the plugin router (built-in
                            // names are reserved and can't be shadowed).
                            // Plugins never stage deferred writes → empty vec.
                            if let Some(mgr) = &self.plugin_router {
                                match mgr.dispatch_tool_call(&call.name, args).await {
                                    Ok(out) => (out, None, true, Vec::new()),
                                    Err(_) => (e.to_string(), None, false, Vec::new()),
                                }
                            } else {
                                (e.to_string(), None, false, Vec::new())
                            }
                        }
                    }
                };

                // P1-c: commit the deferred writes the tool staged — the
                // engine owns the side effect so it lands only after the
                // permission decision. A multi-file tool (`apply_patch`)
                // stages one `PendingWrite` per file op; each op captures
                // its own pre-image for the hunk record (binary-safe).
                for pw in pending_write {
                    let old_content = std::fs::read(&pw.path).ok();
                    match pw.op {
                        crate::tools::registry::WriteOp::Delete => {
                            match tokio::fs::remove_file(&pw.path).await {
                                Ok(()) => {
                                    hunks.record_write(pw.path.clone(), old_content, Vec::new(), "agent");
                                }
                                Err(e) => {
                                    ok = false;
                                    content = format!("delete failed for {}: {e}", pw.path.display());
                                }
                            }
                        }
                        crate::tools::registry::WriteOp::Write => {
                            if pw.auto_mkdir {
                                if let Some(parent) = pw.path.parent() {
                                    let _ = tokio::fs::create_dir_all(parent).await;
                                }
                            }
                            match tokio::fs::write(&pw.path, &pw.content).await {
                                Ok(()) => {
                                    hunks.record_write(pw.path.clone(), old_content, pw.content, "agent");
                                }
                                Err(e) => {
                                    ok = false;
                                    content = format!("write failed for {}: {e}", pw.path.display());
                                }
                            }
                        }
                    }
                }

                // ---- Stage 9: after_tool hooks (may mutate output) ----
                self.hooks.run_after_tool(&call, &mut content).await;

                let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                    name: call.name.clone(),
                    ok,
                    content: content.clone(),
                    ui_type,
                });

                // `ok` survives into the snapshot via `is_error` so a
                // reloaded view replays the red capsule instead of a green
                // one — the live `ToolCallFinished.ok` carried it.
                history.push(if ok {
                    ChatMessage::tool_result(call.id.clone(), content)
                } else {
                    ChatMessage::tool_result_err(call.id.clone(), content)
                });
            }

            // If we have reached or exceeded the tool limit, force the next round to be a synthesis round without tools
            if tool_rounds >= MAX_TOOL_ROUNDS && !force_no_tools {
                let msg = format!("已达单轮工具调用上限 ({MAX_TOOL_ROUNDS} 轮)，正在汇总已收集的信息生成最终回答...");
                let _ = io.ui_tx.try_send(UiEvent::SystemMessage(msg.clone()));
                history.push(ChatMessage::notice(msg));
                history.push(ChatMessage::user_hidden(
                    "You have reached the tool execution limit for this turn. Do NOT request any more tool calls. Please synthesize all your findings and provide your complete, detailed response/answer to the user now.",
                ));
                force_no_tools = true;
            }

            // Tools done → back to sampling for the model's next move.
            self.set_state(io, AgentState::Reasoning);
        }
    }

    /// Two-pass compaction: split history at ~75% suffix → summarize the
    /// prefix into a dense `NOTE` via the current provider → splice the note
    /// in place of the prefix. Pass 2 (condense NOTE + suffix) is implicit —
    /// the note is already compact enough that the suffix fits.
    async fn compact_history(
        &mut self,
        _io: &mut EngineIo,
        history: &mut Vec<ChatMessage>,
    ) -> Result<(), String> {
        let plan = compaction::plan(history, self.context_window)
            .ok_or("history too small to compact")?;

        let prefix_text: String = history[..plan.prefix_end]
            .iter()
            .filter_map(|m| m.content.as_deref())
            .collect::<Vec<_>>()
            .join("\n---\n");
        let mut summarize_history = vec![
            ChatMessage::system(
                "You are a context compactor. Compress the conversation below into a dense \
                 note (facts, decisions, file:line touched, tool outcomes) that preserves \
                 everything needed to continue the task. Output only the note."),
            ChatMessage::user(prefix_text),
        ];

        // One summarize call — no tools, low temperature.
        let mut note = String::new();
        let req = SampleRequest {
            model: &self.model,
            temperature: 0.0,
            tools: None,
            reasoning_effort: None,
        };
        self.sampler
            .sample(
                req,
                &mut summarize_history,
                |chunk| {
                    if let StreamChunk::ContentDelta(t) = chunk {
                        note.push_str(t);
                    }
                },
                |_| {},
            )
            .await
            .map_err(|e| e.to_string())?;

        if note.trim().is_empty() {
            return Err("compactor returned empty note".into());
        }
        compaction::apply(history, &plan, note);
        Ok(())
    }

    /// Poll the shared decision slot until SessionActor writes a verdict
    /// for THIS `request_id`, or the safety timeout fires (fail-closed:
    /// deny on timeout).
    ///
    /// The verdict slot holds `Option<(request_id, approved)>`. A verdict
    /// whose id doesn't match is a stale click on an already-resolved card —
    /// it is cleared and ignored rather than allowed to approve this tool
    /// (P1-a). Also returns `false` early if the user cancels while the
    /// approval is pending — a Cancel must not wait out the 300 s timeout.
    async fn wait_for_decision(
        &self,
        request_id: u64,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> bool {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            // Poison-safe: a panicked writer must not deadlock the engine.
            // Take the verdict out of the slot in a short scope so the
            // MutexGuard is never held across `.await` (not `Send`).
            let verdict = {
                let mut slot = self
                    .decision
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                match *slot {
                    // This request's verdict — consume and return it.
                    Some((id, approved)) if id == request_id => {
                        *slot = None;
                        Some(approved)
                    }
                    // A stale verdict (different request id) — drop it so it
                    // can't accumulate, then keep waiting for OUR verdict.
                    Some(_) => {
                        *slot = None;
                        None
                    }
                    None => None,
                }
            };
            if let Some(approved) = verdict {
                return approved;
            }
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return false; // cancelled while awaiting approval → deny
            }
            if std::time::Instant::now() > deadline {
                return false; // fail-closed
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    fn set_state(&self, io: &EngineIo, state: AgentState) {
        let _ = io.ui_tx.try_send(UiEvent::StateChanged(state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_thinking_level_resolution() {
        let mut engine = Engine::new(
            Arc::new(agent_llm::adapters::MockProvider::new()),
            Arc::new(ToolRegistry::with_builtins()),
            Arc::new(ToolCtx::new(Path::new("."))),
            "devin/swe-2",
            1.0,
        );

        // Without level set
        assert_eq!(engine.resolve_reasoning_effort(), None);

        // Off returns None
        engine.set_thinking_level(Some("off".into()));
        assert_eq!(engine.resolve_reasoning_effort(), None);

        // With default mapping (no map installed)
        engine.set_thinking_level(Some("medium".into()));
        assert_eq!(engine.resolve_reasoning_effort().as_deref(), Some("medium"));

        // With custom thinking level map installed (like Devin SWE-2)
        let mut map = std::collections::HashMap::new();
        map.insert("off".into(), None);
        map.insert("minimal".into(), None);
        map.insert("low".into(), None);
        map.insert("medium".into(), Some("medium".into()));
        map.insert("high".into(), Some("high".into()));
        map.insert("max".into(), Some("max".into()));
        engine.set_thinking_level_map(Some(map));

        engine.set_thinking_level(Some("medium".into()));
        assert_eq!(engine.resolve_reasoning_effort().as_deref(), Some("medium"));

        engine.set_thinking_level(Some("high".into()));
        assert_eq!(engine.resolve_reasoning_effort().as_deref(), Some("high"));

        engine.set_thinking_level(Some("max".into()));
        assert_eq!(engine.resolve_reasoning_effort().as_deref(), Some("max"));

        engine.set_thinking_level(Some("low".into()));
        assert_eq!(engine.resolve_reasoning_effort(), None);

        engine.set_thinking_level(Some("off".into()));
        assert_eq!(engine.resolve_reasoning_effort(), None);
    }
}
