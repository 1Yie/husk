//! `AgentState` machine + the ReAct turn loop.
//!
//! ```text
//! Idle→ScanningWorkspace→Reasoning→StreamingToken
//!     →{Finished | AwaitingToolConfirmation→ExecutingTool→Reasoning}
//! ```
//!
//! `Failed` is terminal per turn, not per session. The loop samples, runs the
//! returned tool calls, appends their results as `tool` messages and re-samples
//! until a turn ends with none pending.

use std::collections::VecDeque;
use std::sync::Arc;

use agent_ipc::{AgentState, UiEvent};
use agent_llm::sampler::{SampleRequest, Sampler, SamplerEvent};
use agent_llm::types::{ChatMessage, StreamChunk, ToolCallAssembler};
use tokio::sync::mpsc;
use tracing::instrument;

use crate::compaction::{self, CompactionSuppressor};
use crate::permissions::{Decision, PermissionGate};
use crate::tools::{ToolCtx, ToolRegistry};

/// The engine's observable side — channels + sink for UI events.
pub struct EngineIo {
    pub ui_tx: crate::channels::UiSink,
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
    /// (prompt, completion, cached) — cached ⊂ prompt.
    pub usage: Option<(u32, u32, u32)>,
    /// Tool calls executed this turn (for hunk attribution / UI list).
    pub tool_calls_run: usize,
}

/// Result of a manual `/compact` — `NothingToDo` is deliberately NOT an
/// error: the user asked, but the history already fits the verbatim suffix
/// budget (or is too short to split), so there is no work rather than a
/// failure to report.
#[derive(Debug)]
pub enum CompactOutcome {
    /// A pass ran and rewrote the history.
    Compacted(compaction::CompactionOutcome),
    /// Nothing worth compacting right now.
    NothingToDo,
}

/// Handle to the live plugin router. `None` = no plugin dir discovered; `Some`
/// with an inner `None` = a plugin manager existed but failed to load, which a
/// later reload can still fill in.
pub type PluginHandle = Arc<std::sync::RwLock<Option<Arc<agent_plugin::PluginManager>>>>;

pub struct Engine {
    sampler: Sampler,
    /// Mode-scoped registries — `set_agent_mode` swaps `registry` between
    /// these. `plan` = readonly view; `goal` = full + goal contract tools.
    registry_full: Arc<ToolRegistry>,
    registry_plan: Arc<ToolRegistry>,
    registry_goal: Arc<ToolRegistry>,
    ctx: Arc<ToolCtx>,
    model: String,
    temperature: f32,
    /// Agent mode — shared slot like `permissions` so a mid-turn composer
    /// switch applies at the next sampling round instead of next session.
    agent_mode: Arc<std::sync::RwLock<crate::mode::AgentMode>>,
    /// Pending mid-turn steering text drained from `io.steer_rx`.
    steer_queue: VecDeque<String>,
    /// Permission gate consulted before every tool dispatch.
    permissions: Arc<std::sync::RwLock<PermissionGate>>,
    /// Decision slot shared with SessionActor — the engine can't hold a
    /// receiver `&mut self`-locked across `run_turn`, so the actor writes
    /// `Some((request_id, approved))` here and the engine polls it. The
    /// `request_id` correlates a verdict to its `ApprovalRequested`; stale
    /// ids are ignored instead of approving a different tool (P1-a).
    decision: Arc<std::sync::Mutex<Option<(u64, bool)>>>,
    /// Monotone id source for `ApprovalRequested` correlation.
    next_request_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Context window for the active model — compaction triggers at
    /// `compact_at` × window (user-settable: 70/80/90%).
    context_window: usize,
    /// Fraction of `context_window` that triggers compaction.
    compact_at: f32,
    /// Per-turn tool-loop bound — sessions use the default; delegated
    /// subagents get a smaller budget.
    max_tool_rounds: usize,
    /// The active model's declared input modalities (`ModelConfig.input`,
    /// e.g. `["text", "image"]`). `"image"` gates whether user-attached
    /// images ride the wire as real parts or degrade to path references.
    model_input: Vec<String>,
    /// Compaction retry-storm suppression.
    compaction_suppressor: CompactionSuppressor,
    /// Last `prompt_tokens` the provider actually reported — a real count
    /// that corrects the chars-based estimate (which under-reads CJK and
    /// tool-call payloads). Reset to the post-compaction estimate after a
    /// successful compact so the stale pre-compact figure can't retrigger.
    last_prompt_tokens: usize,
    /// Ordered hook chain (veto/mutate before+after tools).
    hooks: crate::hooks::HookChain,
    /// Plugin tool router — built-in names win; plugins fill the rest.
    /// Shared with the `SessionManager`, which can REPLACE the manager at
    /// runtime (a plugin added in settings). Read per request, never cached, so
    /// a live session picks up a reload without being restarted.
    plugin_router: Option<PluginHandle>,
    /// Thinking / reasoning intensity level (e.g. "off", "low", "medium",
    /// "high", "max"). Shared slot like `permissions` — the bridge reads it
    /// to report a session's live level back to the UI.
    thinking_level: Arc<std::sync::RwLock<Option<String>>>,
    /// Model-specific thinking level mapping from pi-agent config schema.
    thinking_level_map: Option<std::collections::HashMap<String, Option<String>>>,
    /// Per-model wire settings from config (`maxTokens`, `samplingParams`,
    /// model-level `compat`) — swapped alongside the model on a hot-swap so
    /// the next request carries the new model's own caps.
    model_params: agent_llm::ModelParams,
}

impl Engine {
    pub fn new(
        provider: Arc<dyn agent_llm::LlmProvider>,
        registry: Arc<ToolRegistry>,
        ctx: Arc<ToolCtx>,
        model: impl Into<String>,
        temperature: f32,
    ) -> Self {
        let registry_plan = Arc::new(registry.readonly_only());
        // Goal mode = full set + the completion contract — the model
        // declares `goal_complete`/`goal_blocked` to end its run.
        let mut goal_reg = (*registry).clone();
        goal_reg.register(crate::tools::goal::spec_complete());
        goal_reg.register(crate::tools::goal::spec_blocked());
        let registry_goal = Arc::new(goal_reg);
        Self {
            sampler: Sampler::new(provider),
            registry_full: registry.clone(),
            registry_plan,
            registry_goal,
            ctx,
            model: model.into(),
            temperature,
            agent_mode: Arc::new(std::sync::RwLock::new(crate::mode::AgentMode::Build)),
            steer_queue: VecDeque::new(),
            permissions: Arc::new(std::sync::RwLock::new(PermissionGate::from_mode_str(
                "default",
            ))),
            decision: Arc::new(std::sync::Mutex::new(None)),
            next_request_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
            context_window: 256_000,
            compact_at: crate::compaction::COMPACT_AT,
            max_tool_rounds: 128,
            model_input: Vec::new(),
            compaction_suppressor: CompactionSuppressor::default(),
            last_prompt_tokens: 0,
            hooks: crate::hooks::HookChain::new(),
            plugin_router: None,
            thinking_level: Arc::new(std::sync::RwLock::new(None)),
            thinking_level_map: None,
            model_params: agent_llm::ModelParams::EMPTY,
        }
    }

    /// Install the active model's per-model wire settings (`maxTokens`,
    /// `samplingParams`, model-level `compat`) — the session actor resolves
    /// these from config at spawn and on every model switch.
    pub fn set_model_params(&mut self, params: agent_llm::ModelParams) {
        self.model_params = params;
    }

    /// The active model's per-model wire settings.
    pub fn model_params(&self) -> &agent_llm::ModelParams {
        &self.model_params
    }

    /// Bound the tool loop per turn — default 128; delegated children
    /// get a tighter budget so a runaway subagent can't burn the turn.
    pub fn set_max_tool_rounds(&mut self, rounds: usize) {
        self.max_tool_rounds = rounds;
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
        // Capability gate first — a wire that can't express reasoning (or
        // has it compat-disabled via `supports_reasoning_effort = false`)
        // gets no effort regardless of the thinking level the user picked.
        if !self.sampler.provider().capabilities().reasoning {
            return None;
        }
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
    pub fn set_plugins(&mut self, handle: PluginHandle) {
        self.plugin_router = Some(handle);
    }

    /// The manager currently installed, if any. A read lock, cloned out — the
    /// handle outlives any single reload.
    fn plugins(&self) -> Option<Arc<agent_plugin::PluginManager>> {
        self.plugin_router
            .as_ref()
            .and_then(|h| h.read().ok().and_then(|g| g.clone()))
    }

    /// The request's `tools` array: built-ins, plus every enabled plugin's
    /// exports as `plugin_id__tool`. The names are sanitized by the plugin
    /// layer into the wire's `function.name` charset — an illegal character
    /// here (`:` in the old spelling) makes a strict upstream reject the whole
    /// request as `invalid_argument`. Plan mode is read-only, so plugins sit it
    /// out there — the permission gate treats an unknown tool name as a write,
    /// which is the behaviour we want for MCP calls in every other mode.
    fn request_tools_schema(&self) -> serde_json::Value {
        let builtin = self.active_registry().request_schema();
        if self.plan_mode() {
            return builtin;
        }
        let Some(mgr) = self.plugins() else {
            return builtin;
        };
        let serde_json::Value::Array(mut tools) = builtin else {
            return builtin;
        };
        tools.extend(mgr.exported_tools());
        serde_json::Value::Array(tools)
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

    /// Compaction trigger as a fraction of the window — the settings UI's
    /// 70/80/90% choices write here at actor spawn.
    pub fn set_compact_at(&mut self, frac: f32) {
        self.compact_at = frac.clamp(0.5, 0.95);
    }

    pub fn compact_at(&self) -> f32 {
        self.compact_at
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

    /// Switch the agent mode — swaps the active registry immediately; the
    /// next sampling round sees the new tool set.
    pub fn set_agent_mode(&mut self, mode: crate::mode::AgentMode) {
        if let Ok(mut w) = self.agent_mode.write() {
            *w = mode;
        }
    }

    /// Shared mode slot — the actor polls it in `run_turn` (a mid-turn
    /// `SetAgentMode` writes the slot without needing `&mut Engine`).
    /// True in plan mode — plugins are advertised and dispatched everywhere
    /// else (their calls are treated as writes by the permission gate).
    fn plan_mode(&self) -> bool {
        self.agent_mode
            .read()
            .map(|m| matches!(*m, crate::mode::AgentMode::Plan))
            .unwrap_or(false)
    }

    pub fn agent_mode_writer(&self) -> Arc<std::sync::RwLock<crate::mode::AgentMode>> {
        self.agent_mode.clone()
    }

    /// Current agent mode — the actor reads this for the prompt swap and
    /// the goal contract.
    pub fn agent_mode(&self) -> crate::mode::AgentMode {
        self.agent_mode.read().map(|m| *m).unwrap_or_default()
    }

    /// The registry for the mode currently in the slot — resolved per call
    /// so a mid-turn `SetAgentMode` applies at the very next dispatch.
    fn active_registry(&self) -> Arc<ToolRegistry> {
        let reg = match self.agent_mode() {
            crate::mode::AgentMode::Plan => self.registry_plan.clone(),
            crate::mode::AgentMode::Goal => self.registry_goal.clone(),
            crate::mode::AgentMode::Build => self.registry_full.clone(),
        };
        // Refresh the ctx slot — `batch_execute` dispatches through the
        // same mode-scoped registry this call resolved.
        *self.ctx.active_registry.write().unwrap() = Some(reg.clone());
        reg
    }

    /// The shared decision slot — SessionActor clones it to deliver
    /// `ToolDecision { request_id, approved }` while the engine is
    /// mid-`run_turn`.
    pub fn decision_slot(&self) -> Arc<std::sync::Mutex<Option<(u64, bool)>>> {
        self.decision.clone()
    }

    /// `AnswerQuestion` resolves here — same bypass-the-pump pattern as
    /// `decision_slot`: the command writes directly while the turn's tool
    /// is parked awaiting the answer.
    pub fn ask_channel(&self) -> Arc<crate::tools::registry::AskChannel> {
        self.ctx.ask.clone()
    }

    /// Hot-swap provider for the next turn.
    pub fn set_provider(&mut self, provider: Arc<dyn agent_llm::LlmProvider>) {
        self.sampler.set_provider(provider);
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    /// Hot-swap the active model's declared input modalities
    /// (`ModelConfig.input`) — SetModel passes the fresh list through.
    pub fn set_model_input(&mut self, input: Vec<String>) {
        self.model_input = input;
    }

    /// Does the active model accept image input? `"image"` in `input` —
    /// a missing/empty list means text-only (the honest default; config
    /// authors mark vision explicitly).
    fn supports_images(&self) -> bool {
        self.model_input
            .iter()
            .any(|i| i.eq_ignore_ascii_case("image"))
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
            let _ = io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
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
        // `<attached-image>` markers carry the composer's staged uploads —
        // they ride the wire as real image parts when the model declares
        // vision, else degrade to path references plus a notice so the
        // user knows the attachment was dropped.
        let images = extract_attached_images(&user_text);
        let mut user_msg = ChatMessage::user(user_text);
        if !images.is_empty() {
            if self.supports_images() {
                user_msg = user_msg.with_images(images);
            } else {
                let line = format!(
                    "模型 {} 不支持图像输入 — 已忽略 {} 张图片",
                    self.model,
                    images.len()
                );
                let _ = io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                history.push(ChatMessage::notice(line));
            }
        }
        history.push(user_msg);
        self.set_state(io, AgentState::Reasoning);

        // Sanitize every turn (flatten tool calls, strip reasoning,
        // budget-fit); compact only when the estimate crosses `compact_at`.
        // P1-b: only flatten tool_calls into text for a text-protocol
        // provider — native function-calling needs the structured array
        // intact or every Role::Tool result orphans.
        let native_tc = self.sampler.provider().native_tool_calls();
        compaction::sanitize_for_sample(
            history,
            self.context_window.saturating_mul(9) / 10,
            native_tc,
            self.supports_images(),
        );
        let est = compaction::estimate_tokens(history);
        // The provider's reported prompt_tokens is the better signal when
        // present — the chars-based estimate under-reads CJK history.
        let est = est.max(self.last_prompt_tokens);
        let fill = est as f32 / self.context_window as f32;
        if compaction::should_compact_at(est, self.context_window, self.compact_at)
            && self.compaction_suppressor.check_at_fill(fill)
        {
            self.set_state(io, AgentState::Compacting);
            match self.compact_history(io, history).await {
                Ok(_) => {
                    // The card row + `UiEvent::Compacted` already went out
                    // from `compact_history` — no second system line here
                    // (that duplicate was the whole visible feedback before).
                    self.compaction_suppressor.on_success();
                }
                Err(e) => {
                    self.compaction_suppressor.on_failure();
                    let line = format!("上下文压缩失败: {e}");
                    let _ = io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
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
        // Runaway-loop bound; delegated children get a smaller budget via
        // set_max_tool_rounds.
        let max_tool_rounds = self.max_tool_rounds;
        // Goal-mode pushback counter — how many times the model went
        // quiet without declaring `goal_complete`/`goal_blocked`.
        let mut goal_followups = 0usize;
        const MAX_GOAL_FOLLOWUPS: usize = 8;
        // A fresh turn starts with a clean goal signal — a leftover
        // `goal_complete` from an earlier turn must not short-circuit this
        // one (the contract is per-turn: each goal prompt ends declared).
        self.ctx.goal.reset();
        /// Cancel sentinel — the `Err` value and `Failed` reason (never
        /// rendered verbatim; the status bar localizes `Failed`). The
        /// user-facing line is `CANCEL_TEXT`.
        const CANCEL_ERR: &str = "turn cancelled by user";
        /// The single system line shown for a cancelled turn.
        const CANCEL_TEXT: &str = "已被用户中断";

        loop {
            // P0-C4: cooperative cancel — checked at the top of every
            // sample/tool round so a Cancel lands between model calls even
            // if no chunk is flowing.
            if io.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                self.set_state(io, AgentState::Failed(CANCEL_ERR.into()));
                let _ = io.ui_tx.send(UiEvent::SystemMessage(CANCEL_TEXT.into()));
                history.push(ChatMessage::notice(CANCEL_TEXT));
                return Err(CANCEL_ERR.into());
            }

            self.drain_steering(io, history);
            while let Some(steer) = self.steer_queue.pop_front() {
                // Echo the steer as a user bubble — the live stream never
                // showed it otherwise (the composer doesn't echo), while a
                // reloaded view rebuilt it from history. One render path.
                let _ = io.ui_tx.send(UiEvent::UserPrompt(steer.clone()));
                // Steering can carry `<attached-image>` markers too — same
                // extraction as the turn-start path; a text-only model
                // just keeps the path reference (no mid-turn notice spam).
                let steer_images = extract_attached_images(&steer);
                let mut steer_msg = ChatMessage::user(format!("The user interrupted: {steer}"));
                if !steer_images.is_empty() && self.supports_images() {
                    steer_msg = steer_msg.with_images(steer_images);
                }
                history.push(steer_msg);
            }

            let native_tc = self.sampler.provider().native_tool_calls();
            compaction::sanitize_for_sample(
                history,
                self.context_window.saturating_mul(9) / 10,
                native_tc,
                self.supports_images(),
            );

            let mut assembler = ToolCallAssembler::new();
            let mut round_text = String::new();
            // This round's reasoning trace — accumulated for the *stored*
            // assistant row (the live stream only ever saw the coalesced
            // deltas), so a reopened session replays the same 思考过程 block.
            let mut round_reasoning = String::new();
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
            // Set by `on_event` on `Retrying`, consumed by `on_chunk` on the
            // retried attempt's first chunk. A retry resends the SAME messages
            // and the provider regenerates from scratch, so every per-attempt
            // accumulator must drop the dead attempt: otherwise history
            // persists `partial + regenerated` text, `assembler` finishes
            // attempt-1's half-written tool calls inside attempt 2, and a
            // stale `stream_error` fails a turn whose retry actually
            // succeeded. Lazily (not at event time) so a retry that never
            // produces a chunk still keeps the partial as salvage. Atomic
            // rather than Cell: the boxed future needs Send.
            let retry_reset = std::sync::atomic::AtomicBool::new(false);

            let effort = self.resolve_reasoning_effort();
            let schema = self.request_tools_schema();
            let tools_opt = if force_no_tools { None } else { Some(&schema) };
            let req = SampleRequest {
                model: &self.model,
                temperature: self.temperature,
                tools: tools_opt,
                reasoning_effort: effort.as_deref(),
                params: &self.model_params,
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
                        if retry_reset.swap(false, std::sync::atomic::Ordering::Relaxed) {
                            round_text.clear();
                            round_reasoning.clear();
                            pending_text_delta.clear();
                            pending_reasoning_delta.clear();
                            assembler = ToolCallAssembler::new();
                            saw_done = false;
                            stream_error = None;
                            usage = None;
                        }
                        match chunk {
                            StreamChunk::ReasoningDelta(t) => {
                                round_reasoning.push_str(t);
                                pending_reasoning_delta.push_str(t);
                                if pending_reasoning_delta.len() >= 120 {
                                    let _ = io.ui_tx.send(UiEvent::ReasoningDelta {
                                        text: std::mem::take(&mut pending_reasoning_delta),
                                        parent: None,
                                    });
                                }
                            }
                            StreamChunk::ContentDelta(t) => {
                                round_text.push_str(t);
                                pending_text_delta.push_str(t);
                                if pending_text_delta.len() >= 120 {
                                    let _ = io.ui_tx.send(UiEvent::TextDelta {
                                        text: std::mem::take(&mut pending_text_delta),
                                        parent: None,
                                    });
                                }
                            }
                            StreamChunk::ToolCallDelta { .. } => {
                                assembler.feed(chunk);
                            }
                            StreamChunk::Done { prompt_tokens, completion_tokens, cached_tokens } => {
                                saw_done = true;
                                // Flush any coalesced deltas BEFORE the
                                // terminal event lands so the UI never
                                // renders trailing text after Finished.
                                if !pending_text_delta.is_empty() {
                                    let _ = io.ui_tx.send(UiEvent::TextDelta {
                                        text: std::mem::take(&mut pending_text_delta),
                                        parent: None,
                                    });
                                }
                                if !pending_reasoning_delta.is_empty() {
                                    let _ = io.ui_tx.send(UiEvent::ReasoningDelta {
                                        text: std::mem::take(&mut pending_reasoning_delta),
                                        parent: None,
                                    });
                                }
                                if let (Some(p), Some(c)) = (prompt_tokens, completion_tokens) {
                                    let cached = cached_tokens.unwrap_or(0);
                                    usage = Some((*p, *c, cached));
                                    // Real count from the wire — feeds the
                                    // compaction trigger next turn so the
                                    // estimate stops being the only signal.
                                    self.last_prompt_tokens = *p as usize;
                                    let _ = io.ui_tx.send(UiEvent::Usage {
                                        prompt_tokens: *p,
                                        completion_tokens: *c,
                                        context_window: self.context_window as u32,
                                        cached_tokens: cached,
                                    });
                                }
                            }
                            StreamChunk::Error(e) => {
                                stream_error = Some(e.clone());
                                if !pending_text_delta.is_empty() {
                                    let _ = io.ui_tx.send(UiEvent::TextDelta {
                                        text: std::mem::take(&mut pending_text_delta),
                                        parent: None,
                                    });
                                }
                                if !pending_reasoning_delta.is_empty() {
                                    let _ = io.ui_tx.send(UiEvent::ReasoningDelta {
                                        text: std::mem::take(&mut pending_reasoning_delta),
                                        parent: None,
                                    });
                                }
                                // The `*[error: e]*` marker + turn-end `Error` event
                                // report this — no extra SystemMessage (it printed
                                // the same text a second time live).
                            }
                        }
                    },
                    |ev| {
                        if matches!(ev, SamplerEvent::Retrying { .. }) {
                            retry_reset.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        let t = ev.to_string();
                        let _ = io.ui_tx.send(UiEvent::SystemMessage(t.clone()));
                        policy_notes.push(t);
                    },
                ) => r,
            };

            // Flush any sub-threshold deltas left after the sample
            // returned — a stream that ended cleanly before hitting 120
            // chars would otherwise strand its tail in the buffer.
            if !pending_text_delta.is_empty() {
                let _ = io.ui_tx.send(UiEvent::TextDelta {
                    text: std::mem::take(&mut pending_text_delta),
                    parent: None,
                });
            }
            if !pending_reasoning_delta.is_empty() {
                let _ = io.ui_tx.send(UiEvent::ReasoningDelta {
                    text: std::mem::take(&mut pending_reasoning_delta),
                    parent: None,
                });
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
                    history.push(
                        ChatMessage::assistant(round_text.clone())
                            .with_reasoning(Some(std::mem::take(&mut round_reasoning))),
                    );
                }
                history.push(ChatMessage::notice(CANCEL_TEXT));
                self.set_state(io, AgentState::Failed(CANCEL_ERR.into()));
                let _ = io.ui_tx.send(UiEvent::SystemMessage(CANCEL_TEXT.into()));
                return Err(CANCEL_ERR.into());
            }

            if let Err(e) = res {
                // Stream salvage (hardening §4): partial text + interrupt
                // line + error line all persist — replay matches the live
                // stream line-for-line.
                if !round_text.is_empty() {
                    history.push(
                        ChatMessage::assistant(round_text.clone())
                            .with_reasoning(Some(std::mem::take(&mut round_reasoning))),
                    );
                    let line = "流传输中断 — 已保留部分内容";
                    let _ = io.ui_tx.send(UiEvent::SystemMessage(line.into()));
                    history.push(ChatMessage::notice(line));
                }
                self.set_state(io, AgentState::Failed(e.to_string()));
                let _ = io.ui_tx.send(UiEvent::Error(e.to_string()));
                history.push(ChatMessage::notice_error(e.to_string()));
                return Err(e.to_string());
            }
            // Provider surfaced an in-stream error (upstream 5xx, rate limit,
            // model refusal, …) — treat the turn as failed. Salvage the
            // partial text and persist the error line (the `⚠` the live
            // stream drew) so a reloaded view loses nothing.
            if let Some(e) = stream_error {
                if !round_text.is_empty() {
                    history.push(
                        ChatMessage::assistant(round_text.clone())
                            .with_reasoning(Some(std::mem::take(&mut round_reasoning))),
                    );
                }
                history.push(ChatMessage::notice_error(e.clone()));
                self.set_state(io, AgentState::Failed(e.clone()));
                let _ = io.ui_tx.send(UiEvent::Error(e.clone()));
                return Err(e);
            }
            debug_assert!(saw_done, "sampler guarantees Done exactly once");

            text.push_str(&round_text);
            let reasoning = std::mem::take(&mut round_reasoning);
            let mut calls = assembler.finish();

            // Degenerate calls — a truncated/announcement delta can produce
            // a slot with a real provider `id` but no `name` (deepseek sends
            // id-only `tool_calls` deltas). Persisting it in the assistant
            // row's `tool_calls` while skipping dispatch orphans the entry:
            // strict backends reject every later request with
            // `No tool output found for tool call …` (400). Drop the call
            // BEFORE the assistant row is built so no dangling id ever
            // reaches the wire — and `name:""` is itself malformed anyway.
            let bad_calls = calls.iter().filter(|c| c.name.trim().is_empty()).count();
            if bad_calls > 0 {
                calls.retain(|c| !c.name.trim().is_empty());
                let line = format!("已跳过 {bad_calls} 个格式异常的空工具调用");
                let _ = io.ui_tx.send(UiEvent::SystemMessage(line.clone()));
                history.push(ChatMessage::notice(line));
            }

            if calls.is_empty() {
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

                // Goal contract — in `goal` mode going quiet undeclared is
                // not a finish: the model must call `goal_complete` /
                // `goal_blocked`. Push it back to work (capped — a model
                // that can't converge shouldn't spin forever).
                if self.agent_mode() == crate::mode::AgentMode::Goal
                    && self.ctx.goal.state() == crate::tools::goal::GoalState::Running
                    && goal_followups < MAX_GOAL_FOLLOWUPS
                {
                    goal_followups += 1;
                    if !final_text.is_empty() {
                        history.push(
                            ChatMessage::assistant(final_text).with_reasoning(Some(reasoning)),
                        );
                    }
                    let nudge =
                        "目标尚未宣告完成 — 继续推进；确实无法前进时调用 `goal_blocked` 说明阻塞。";
                    history.push(ChatMessage::user_hidden(nudge));
                    let _ = io.ui_tx.send(UiEvent::SystemMessage(nudge.into()));
                    continue;
                }

                history.push(ChatMessage {
                    role: agent_llm::Role::Assistant,
                    content: if final_text.is_empty() {
                        None
                    } else {
                        Some(final_text.clone())
                    },
                    tool_calls: None,
                    tool_call_id: None,
                    is_error: None,
                    notice: None,
                    ts: Some(agent_llm::types::now_ms()),
                    images: Vec::new(),
                    reasoning: Some(reasoning),
                });
                self.set_state(io, AgentState::Finished);
                let _ = io.ui_tx.send(UiEvent::AssistantMessage(final_text));
                return Ok(TurnOutcome {
                    text,
                    usage,
                    tool_calls_run,
                });
            }

            history.push(ChatMessage {
                role: agent_llm::Role::Assistant,
                content: if round_text.is_empty() {
                    None
                } else {
                    Some(round_text.clone())
                },
                tool_calls: Some(calls.clone()),
                tool_call_id: None,
                is_error: None,
                notice: None,
                ts: Some(agent_llm::types::now_ms()),
                images: Vec::new(),
                reasoning: Some(reasoning),
            });

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
                    let _ = io.ui_tx.send(UiEvent::SystemMessage(CANCEL_TEXT.into()));
                    return Err(CANCEL_ERR.into());
                }
                // Skip degenerate calls — a text-protocol echo can produce an
                // empty-name or empty-args call that must never dispatch
                // (it'd surface as `unknown tool ''` and poison history).
                if call.name.trim().is_empty() {
                    // Unreachable since the pre-persist filter above — kept
                    // as a pairing guard: a skipped call must still emit a
                    // tool row, or its `tool_calls` entry dangles and strict
                    // providers reject the next request (deepseek 400).
                    let msg = "skipped malformed empty tool call".to_string();
                    let _ = io.ui_tx.send(UiEvent::ToolCallFinished {
                        name: call.name.clone(),
                        ok: false,
                        content: msg.clone(),
                        ui_type: None,
                        parent: None,
                    });
                    history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                    continue;
                }
                tool_calls_run += 1;

                let args: serde_json::Value = serde_json::from_str(&call.arguments)
                    .unwrap_or_else(|_| serde_json::json!({ "_malformed": call.arguments }));

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
                                            let prefix: String =
                                                chars.by_ref().take(4096).collect();
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
                let _ = io.ui_tx.send(UiEvent::ToolCallStarted {
                    name: call.name.clone(),
                    args_preview,
                    parent: None,
                });
                self.set_state(
                    io,
                    AgentState::ExecutingTool {
                        tool_name: call.name.clone(),
                    },
                );

                // before_tool hooks (veto/mutate, pre-gate)
                let mut call_mut = call.clone();
                if !self.hooks.run_before_tool(&mut call_mut).await {
                    let msg = format!("tool `{}` vetoed by hook", call.name);
                    let _ = io.ui_tx.send(UiEvent::ToolCallFinished {
                        name: call.name.clone(),
                        ok: false,
                        content: msg.clone(),
                        ui_type: None,
                        parent: None,
                    });
                    history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                    continue;
                }
                let call = &call_mut; // hooks may have rewritten args

                // Permission gate before dispatch
                let is_readonly = self.active_registry().is_readonly(&call.name);
                let shell_cmd = args.get("command").and_then(|v| v.as_str());
                let diff_summary = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|p| format!("{p}"))
                    .unwrap_or_else(|| call.name.clone());

                // P1-c: write tools (fuzzy_patch/apply_patch/write_file) are
                // dispatched EARLY — they return a `PendingWrite` instead of
                // writing inside the tool, so we can show the REAL diff +
                // fuzzy flag on the approval card and only commit the write
                // after the user approves. Read-only/bash tools keep the
                // classic gate→dispatch flow (they have side effects we
                // can't dry-run).
                let is_write = matches!(
                    call.name.as_str(),
                    "fuzzy_patch" | "apply_patch" | "write_file"
                );

                // Dry-run a write tool to harvest diff/fuzzy/pending_write.
                // `dispatch` on these tools is side-effect-free (returns
                // PendingWrite), so it's safe to run pre-approval.
                let staged: Option<crate::tools::registry::ToolResult> = if is_write {
                    match self
                        .active_registry()
                        .dispatch(&call.name, args.clone(), self.ctx.clone())
                        .await
                    {
                        Ok(res) => Some(res),
                        Err(e) => {
                            let msg = e.to_string();
                            let _ = io.ui_tx.send(UiEvent::ToolCallFinished {
                                name: call.name.clone(),
                                ok: false,
                                content: msg.clone(),
                                ui_type: None,
                                parent: None,
                            });
                            history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                            continue;
                        }
                    }
                } else {
                    None
                };
                let real_diff = staged
                    .as_ref()
                    .map(|r| r.content.clone())
                    .unwrap_or_else(|| {
                        args.get("replace")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    });
                let real_fuzzy = staged.as_ref().map(|r| r.fuzzy).unwrap_or(false);

                let decision = self
                    .permissions
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .decide(&call.name, is_readonly, shell_cmd, diff_summary);

                match decision {
                    Decision::Deny { reason } => {
                        let _ = io.ui_tx.send(UiEvent::ToolCallFinished {
                            name: call.name.clone(),
                            ok: false,
                            content: reason.clone(),
                            ui_type: None,
                            parent: None,
                        });
                        history.push(ChatMessage::tool_result_err(call.id.clone(), reason));
                        continue;
                    }
                    Decision::Ask { diff_summary } => {
                        // Pause in AwaitingToolConfirmation; the UI's
                        // ToolDecision writes the shared slot via
                        // decision_slot(). P1-a: a fresh request_id ties
                        // this pending approval to its verdict — a stale
                        // decision can't approve this tool.
                        let request_id = self
                            .next_request_id
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        self.set_state(
                            io,
                            AgentState::AwaitingToolConfirmation {
                                tool_name: call.name.clone(),
                                diff_summary: diff_summary.clone(),
                            },
                        );
                        // P1-c: show the REAL diff + fuzzy flag the staged
                        // run produced — the card reflects what gets written.
                        let _ = io.ui_tx.send(UiEvent::ApprovalRequested {
                            request_id,
                            tool_name: call.name.clone(),
                            diff: real_diff.clone(),
                            fuzzy: real_fuzzy,
                        });
                        let approved = self.wait_for_decision(request_id, &io.cancel).await;
                        self.set_state(
                            io,
                            AgentState::ExecutingTool {
                                tool_name: call.name.clone(),
                            },
                        );
                        if !approved {
                            let msg = format!("user denied {call_name}", call_name = call.name);
                            let _ = io.ui_tx.send(UiEvent::ToolCallFinished {
                                name: call.name.clone(),
                                ok: false,
                                content: msg.clone(),
                                ui_type: None,
                                parent: None,
                            });
                            history.push(ChatMessage::tool_result_err(call.id.clone(), msg));
                            continue;
                        }
                    }
                    Decision::Allow => {}
                }

                // Staged write tools already hold their result; dispatch the
                // rest now (read-only/bash).
                let (mut content, ui_type, mut ok, pending_write) = if let Some(res) = staged {
                    let ui_type = res.ui_type.map(|s| s.to_string());
                    (res.content, ui_type, true, res.pending_write)
                } else {
                    match self
                        .active_registry()
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
                            // Plan mode dispatches read-only built-ins only.
                            if self.plan_mode() {
                                (e.to_string(), None, false, Vec::new())
                            } else if let Some(mgr) = self.plugins() {
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
                                    hunks.record_write(
                                        pw.path.clone(),
                                        old_content,
                                        Vec::new(),
                                        "agent",
                                    );
                                }
                                Err(e) => {
                                    ok = false;
                                    content =
                                        format!("delete failed for {}: {e}", pw.path.display());
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
                                    hunks.record_write(
                                        pw.path.clone(),
                                        old_content,
                                        pw.content,
                                        "agent",
                                    );
                                }
                                Err(e) => {
                                    ok = false;
                                    content =
                                        format!("write failed for {}: {e}", pw.path.display());
                                }
                            }
                        }
                    }
                }

                // after_tool hooks (may mutate output)
                self.hooks.run_after_tool(&call, &mut content).await;

                let _ = io.ui_tx.send(UiEvent::ToolCallFinished {
                    name: call.name.clone(),
                    ok,
                    content: content.clone(),
                    ui_type,
                    parent: None,
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
            if tool_rounds >= max_tool_rounds && !force_no_tools {
                let msg = format!("已达单轮工具调用上限 ({max_tool_rounds} 轮)，正在汇总已收集的信息生成最终回答...");
                let _ = io.ui_tx.send(UiEvent::SystemMessage(msg.clone()));
                history.push(ChatMessage::notice(msg));
                history.push(ChatMessage::user_hidden(
                    "You have reached the tool execution limit for this turn. Do NOT request any more tool calls. Please synthesize all your findings and provide your complete, detailed response/answer to the user now.",
                ));
                force_no_tools = true;
            }

            self.set_state(io, AgentState::Reasoning);
        }
    }

    /// Compaction: split history keeping ~25% as the verbatim suffix →
    /// summarize the prefix into a dense `NOTE` via the current provider →
    /// splice the note in place of the prefix. A prior NOTE folds into the
    /// new one (the compactor is told to merge, not stack).
    async fn compact_history(
        &mut self,
        io: &mut EngineIo,
        history: &mut Vec<ChatMessage>,
    ) -> Result<Option<compaction::CompactionOutcome>, String> {
        let before_tokens = compaction::estimate_tokens(history);
        // `None` = nothing to do (too few messages, or the whole history
        // already fits the ~25% suffix budget). Not an error — the caller
        // decides how to report it.
        let Some(plan) = compaction::plan(history, self.context_window) else {
            return Ok(None);
        };

        // `history[0]` is the rendered kernel system prompt — it survives
        // compaction verbatim, so feeding it to the compactor only burns
        // input tokens and risks protocol language leaking into the note.
        // `NoticeKind::Compacted` rows are UI cards, not conversation —
        // feeding their JSON payload to the compactor is pure noise.
        let prefix_text: String = history[1..plan.prefix_end]
            .iter()
            .filter(|m| m.notice != Some(agent_llm::types::NoticeKind::Compacted))
            .map(compaction::render_for_summary)
            .collect::<Vec<_>>()
            .join("\n---\n");
        let summarize_history = vec![
            ChatMessage::system(
                "You are a context compactor. Compress the conversation below into a dense \
                 note (facts, decisions, file:line touched, tool outcomes) that preserves \
                 everything needed to continue the task. If the input already contains a \
                 [BEGIN COMPACTED CONTEXT] note, merge its content into the new note — do \
                 not stack summaries. Output only the note.",
            ),
            ChatMessage::user(prefix_text),
        ];

        let mut note = String::new();
        // Same retry-reset rule as the main loop: a retry regenerates the
        // summary from scratch, so the dead attempt's text must not survive
        // into `note`.
        let retry_reset = std::sync::atomic::AtomicBool::new(false);
        let req = SampleRequest {
            model: &self.model,
            temperature: 0.0,
            tools: None,
            reasoning_effort: None,
            // Summarization is a real request on the same model — it must
            // respect the same output cap and per-model compat.
            params: &self.model_params,
        };
        self.sampler
            .sample(
                req,
                &summarize_history,
                |chunk| {
                    if retry_reset.swap(false, std::sync::atomic::Ordering::Relaxed) {
                        note.clear();
                    }
                    if let StreamChunk::ContentDelta(t) = chunk {
                        note.push_str(t);
                    }
                },
                |ev| {
                    if matches!(ev, SamplerEvent::Retrying { .. }) {
                        retry_reset.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                },
            )
            .await
            .map_err(|e| e.to_string())?;

        if note.trim().is_empty() {
            return Err("compactor returned empty note".into());
        }
        let removed_messages = plan.prefix_end.saturating_sub(1);
        compaction::apply(history, &plan, note.clone());
        // The pre-compact `prompt_tokens` is stale — reseed the actual-usage
        // signal with the post-splice estimate or the trigger would
        // immediately re-fire on the stale (large) figure.
        let after_tokens = compaction::estimate_tokens(history);
        self.last_prompt_tokens = after_tokens;
        // Persist the display row at the point in the stream where the
        // compaction happened (mid-turn: right after the live user turn) —
        // replay then draws the card in the same place the live event did.
        history.push(ChatMessage::compaction(
            before_tokens as u32,
            after_tokens as u32,
            removed_messages as u32,
            &note,
        ));
        // Card + meter in one event: the stream gets the summary, the
        // title-bar fill drops to the post-splice figure now instead of at
        // the next sample's `Usage`.
        let _ = io.ui_tx.send(UiEvent::Compacted {
            before_tokens: before_tokens as u32,
            after_tokens: after_tokens as u32,
            removed_messages: removed_messages as u32,
            context_window: self.context_window as u32,
            note: note.clone(),
        });
        Ok(Some(compaction::CompactionOutcome {
            before_tokens,
            after_tokens,
            removed_messages,
            note,
        }))
    }

    /// Manual `/compact` — bypasses the threshold AND the suppressor (the
    /// user asked for it now), but a result still updates the suppression
    /// state so a failed manual attempt doesn't leave the storm guard off.
    pub async fn compact_now(
        &mut self,
        io: &mut EngineIo,
        history: &mut Vec<ChatMessage>,
    ) -> Result<CompactOutcome, String> {
        match self.compact_history(io, history).await {
            Ok(Some(outcome)) => {
                self.compaction_suppressor.on_success();
                Ok(CompactOutcome::Compacted(outcome))
            }
            Ok(None) => Ok(CompactOutcome::NothingToDo),
            Err(e) => {
                self.compaction_suppressor.on_failure();
                Err(e)
            }
        }
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
                let mut slot = self.decision.lock().unwrap_or_else(|e| e.into_inner());
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
        let _ = io.ui_tx.send(UiEvent::StateChanged(state));
    }
}

/// Pull `<attached-image path="…"/>` markers out of a prompt — the
/// composer's image-attachment channel, mirroring `<attached-file>`.
/// The marker itself stays in `content` (path provenance the model can
/// act on with `read`/`bash`); the returned refs become real wire parts
/// when the model declares `"image"` input.
fn extract_attached_images(text: &str) -> Vec<agent_llm::types::ImageRef> {
    const TAG: &str = "<attached-image path=\"";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(TAG) {
        let after = &rest[start + TAG.len()..];
        let Some(end) = after.find('"') else { break };
        if let Some(img) =
            agent_llm::types::ImageRef::for_path(std::path::PathBuf::from(&after[..end]))
        {
            out.push(img);
        }
        rest = &after[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_thinking_level_resolution() {
        let mut engine = Engine::new(
            Arc::new(agent_llm::provider::UnconfiguredProvider),
            Arc::new(ToolRegistry::with_builtins()),
            Arc::new(ToolCtx::new(Path::new("."))),
            "devin/swe-2",
            1.0,
        );

        assert_eq!(engine.resolve_reasoning_effort(), None);

        engine.set_thinking_level(Some("off".into()));
        assert_eq!(engine.resolve_reasoning_effort(), None);

        engine.set_thinking_level(Some("medium".into()));
        assert_eq!(engine.resolve_reasoning_effort().as_deref(), Some("medium"));

        // Custom thinking-level map (like Devin SWE-2).
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

    #[test]
    fn extracts_attached_image_markers() {
        let text = "看看这张图\n\n<attached-image path=\"/ws/.husk/attachments/cat-a1b2.png\" name=\"cat.png\"/>\n\n<attached-image path=\"/ws/.husk/attachments/doc-c3d4.txt\"/>";
        let imgs = extract_attached_images(text);
        assert_eq!(imgs.len(), 1); // .txt isn't an image type
        assert_eq!(imgs[0].media_type, "image/png");
        assert!(imgs[0].path.ends_with("cat-a1b2.png"));
        assert!(extract_attached_images("plain text").is_empty());
    }
}
