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
    permissions: PermissionGate,
    /// Decision slot shared with SessionActor — the engine can't hold a
    /// receiver `&mut self`-locked across `run_turn`, so the actor writes
    /// `Some(approved)` here and the engine polls it between polls.
    decision: Arc<std::sync::Mutex<Option<bool>>>,
    /// Context window for the active model — compaction triggers at 80%.
    context_window: usize,
    /// Stage 7: compaction retry-storm suppression.
    compaction_suppressor: CompactionSuppressor,
    /// Stage 9: ordered hook chain (veto/mutate before+after tools).
    hooks: crate::hooks::HookChain,
    /// Plugin tool router — built-in names win; plugins fill the rest.
    plugin_router: Option<Arc<agent_plugin::PluginManager>>,
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
            permissions: PermissionGate::from_mode_str("default"),
            decision: Arc::new(std::sync::Mutex::new(None)),
            context_window: 256_000,
            compaction_suppressor: CompactionSuppressor::default(),
            hooks: crate::hooks::HookChain::new(),
            plugin_router: None,
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

    /// SessionActor installs the permission gate (mode + repo rules).
    pub fn set_permissions(&mut self, gate: PermissionGate) {
        self.permissions = gate;
    }

    /// The shared decision slot — SessionActor clones it to deliver
    /// `ToolDecision` while the engine is mid-`run_turn`.
    pub fn decision_slot(&self) -> Arc<std::sync::Mutex<Option<bool>>> {
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
    fn drain_steering(&mut self, io: &mut EngineIo) {
        while let Ok(text) = io.steer_rx.try_recv() {
            self.steer_queue.push_back(text);
        }
        if !self.steer_queue.is_empty() {
            // Notify the UI the turn was steered (provenance marker).
            let _ = io.ui_tx.try_send(UiEvent::SystemMessage(
                "[steered] steering input queued".into()));
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
        compaction::sanitize_for_sample(history, self.context_window.saturating_mul(9) / 10);
        let est = compaction::estimate_tokens(history);
        if compaction::should_compact(est, self.context_window)
            && self.compaction_suppressor.check()
        {
            self.set_state(io, AgentState::Compacting);
            match self.compact_history(io, history).await {
                Ok(()) => {
                    self.compaction_suppressor.on_success();
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(
                        format!("compacted history (was ~{est} tokens)")));
                }
                Err(e) => {
                    self.compaction_suppressor.on_failure();
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(
                        format!("compaction failed: {e}")));
                }
            }
            self.set_state(io, AgentState::Reasoning);
        }

        let mut text = String::new();
        let mut usage = None;
        let mut tool_calls_run = 0usize;
        let mut tool_rounds = 0usize;
        const MAX_TOOL_ROUNDS: usize = 16; // guard against infinite tool loops

        loop {
            // Inject any mid-turn steering as a user message before sampling.
            self.drain_steering(io);
            while let Some(steer) = self.steer_queue.pop_front() {
                history.push(ChatMessage::user(format!("The user interrupted: {steer}")));
            }

            let mut assembler = ToolCallAssembler::new();
            let mut round_text = String::new();
            let mut saw_done = false;
            // A provider-level `StreamChunk::Error` (e.g. devin's
            // `internal_server_error … upstream error`) arrives inside the
            // stream, not as a transport `Err` — without this flag the turn
            // would fall through to the no-calls branch and persist an EMPTY
            // assistant message into history, corrupting every later request.
            let mut stream_error: Option<String> = None;

            let req = SampleRequest {
                model: &self.model,
                temperature: self.temperature,
                tools: Some(&self.registry.request_schema()),
            };

            let res = self
                .sampler
                .sample(
                    req,
                    history,
                    |chunk| {
                        match chunk {
                            StreamChunk::ReasoningDelta(t) => {
                                let _ = io.ui_tx.try_send(UiEvent::ReasoningDelta(t.clone()));
                            }
                            StreamChunk::ContentDelta(t) => {
                                round_text.push_str(t);
                                let _ = io.ui_tx.try_send(UiEvent::TextDelta(t.clone()));
                            }
                            StreamChunk::ToolCallDelta { .. } => {
                                assembler.feed(chunk);
                            }
                            StreamChunk::Done { prompt_tokens, completion_tokens } => {
                                saw_done = true;
                                if let (Some(p), Some(c)) = (prompt_tokens, completion_tokens) {
                                    usage = Some((*p, *c));
                                    let _ = io.ui_tx.try_send(UiEvent::Usage {
                                        prompt_tokens: *p,
                                        completion_tokens: *c,
                                    });
                                }
                            }
                            StreamChunk::Error(e) => {
                                stream_error = Some(e.clone());
                                let _ = io.ui_tx.try_send(UiEvent::SystemMessage(e.clone()));
                            }
                        }
                    },
                    |ev| {
                        let _ = io
                            .ui_tx
                            .try_send(UiEvent::SystemMessage(format!("{ev:?}")));
                    },
                )
                .await;

            if let Err(e) = res {
                // ---- Stream salvage (hardening §4) ----
                // Partial text stays visible; mark it `interrupted` so the
                // UI renders a cut-off banner instead of discarding.
                if !round_text.is_empty() {
                    history.push(ChatMessage {
                        role: agent_llm::Role::Assistant,
                        content: Some(format!("{round_text}\n\n*[interrupted — transport error]*")),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(
                        "stream interrupted — partial response preserved".into(),
                    ));
                }
                self.set_state(io, AgentState::Failed(e.to_string()));
                let _ = io.ui_tx.try_send(UiEvent::Error(e.to_string()));
                return Err(e.to_string());
            }
            // Provider surfaced an in-stream error (upstream 5xx, rate limit,
            // model refusal, …) — treat the turn as failed. Do NOT persist an
            // assistant message: an empty content would replay into the next
            // request as a blank turn and poison the context.
            if let Some(e) = stream_error {
                self.set_state(io, AgentState::Failed(e.clone()));
                let _ = io.ui_tx.try_send(UiEvent::Error(e.clone()));
                return Err(e);
            }
            debug_assert!(saw_done, "sampler guarantees Done exactly once");

            text.push_str(&round_text);
            let calls = assembler.finish();

            if calls.is_empty() {
                // No tool calls → turn complete. Persist the final assistant
                // message so the next turn sees it in history.
                history.push(ChatMessage {
                    role: agent_llm::Role::Assistant,
                    content: if round_text.is_empty() { None } else { Some(round_text.clone()) },
                    tool_calls: None,
                    tool_call_id: None,
                });
                self.set_state(io, AgentState::Finished);
                let _ = io.ui_tx.try_send(UiEvent::AssistantMessage(round_text));
                return Ok(TurnOutcome { text, usage, tool_calls_run });
            }

            // Append the assistant turn with its tool calls.
            history.push(ChatMessage {
                role: agent_llm::Role::Assistant,
                content: if round_text.is_empty() { None } else { Some(round_text.clone()) },
                tool_calls: Some(calls.clone()),
                tool_call_id: None,
            });

            // Execute each tool call.
            tool_rounds += 1;
            if tool_rounds > MAX_TOOL_ROUNDS {
                self.set_state(io, AgentState::Failed("tool loop guard".into()));
                return Err(format!(
                    "exceeded {MAX_TOOL_ROUNDS} tool rounds — likely a stuck model"
                ));
            }

            for call in calls {
                // Skip degenerate calls — a text-protocol echo can produce an
                // empty-name or empty-args call that must never dispatch
                // (it'd surface as `unknown tool ''` and poison history).
                if call.name.trim().is_empty() {
                    let _ = io.ui_tx.try_send(UiEvent::SystemMessage(
                        "skipped a malformed empty tool call".into(),
                    ));
                    continue;
                }
                tool_calls_run += 1;
                let _ = io.ui_tx.try_send(UiEvent::ToolCallStarted {
                    name: call.name.clone(),
                });
                self.set_state(io, AgentState::ExecutingTool {
                    tool_name: call.name.clone(),
                });

                let args: serde_json::Value =
                    serde_json::from_str(&call.arguments).unwrap_or_else(|_| {
                        serde_json::json!({ "_malformed": call.arguments })
                    });

                // ---- Stage 9: before_tool hooks (veto/mutate, pre-gate) ----
                let mut call_mut = call.clone();
                if !self.hooks.run_before_tool(&mut call_mut).await {
                    let msg = format!("tool `{}` vetoed by hook", call.name);
                    let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                        name: call.name.clone(), ok: false,
                        content: msg.clone(), ui_type: None,
                    });
                    history.push(ChatMessage::tool_result(call.id.clone(), msg));
                    continue;
                }
                let call = &call_mut; // hooks may have rewritten args

                // ---- Stage 6: permission gate before dispatch ----
                let is_readonly = self.registry.is_readonly(&call.name);
                let shell_cmd = args.get("command").and_then(|v| v.as_str());
                let diff_summary = args.get("path").and_then(|v| v.as_str())
                    .map(|p| format!("{p}")).unwrap_or_else(|| call.name.clone());

                match self.permissions.decide(&call.name, is_readonly, shell_cmd, diff_summary) {
                    Decision::Deny { reason } => {
                        let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                            name: call.name.clone(), ok: false,
                            content: reason.clone(), ui_type: None,
                        });
                        history.push(ChatMessage::tool_result(call.id.clone(), reason));
                        continue; // skip dispatch — denied by policy
                    }
                    Decision::Ask { diff_summary } => {
                        // Pause in AwaitingToolConfirmation; the UI's
                        // ToolDecision resolves it. Wait on the shared slot
                        // (SessionActor writes it via decision_slot()).
                        self.set_state(io, AgentState::AwaitingToolConfirmation {
                            tool_name: call.name.clone(),
                            diff_summary: diff_summary.clone(),
                        });
                        let _ = io.ui_tx.try_send(UiEvent::ApprovalRequested {
                            tool_name: call.name.clone(),
                            diff: args.get("replace").and_then(|v| v.as_str())
                                .unwrap_or("").to_string(),
                            fuzzy: false,
                        });
                        let approved = self.wait_for_decision().await;
                        self.set_state(io, AgentState::ExecutingTool {
                            tool_name: call.name.clone(),
                        });
                        if !approved {
                            let msg = format!("user denied {call_name}", call_name = call.name);
                            let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                                name: call.name.clone(), ok: false,
                                content: msg.clone(), ui_type: None,
                            });
                            history.push(ChatMessage::tool_result(call.id.clone(), msg));
                            continue;
                        }
                    }
                    Decision::Allow => {}
                }

                // Stage 6: capture pre-write content for hunk recording
                // (only for write tools — read-only tools never touch disk).
                let write_path = args.get("path").and_then(|v| v.as_str()).map(|s| s.to_string());
                let is_write = matches!(call.name.as_str(),
                    "fuzzy_patch" | "apply_patch" | "write_file");
                let old_content = if is_write {
                    write_path.as_deref().and_then(|p| {
                        self.ctx.resolve(p).ok()
                            .and_then(|abs| std::fs::read_to_string(abs).ok())
                    })
                } else {
                    None
                };

                let (mut content, ui_type, ok) = match self
                    .registry
                    .dispatch(&call.name, args.clone(), self.ctx.clone())
                    .await
                {
                    Ok(res) => (res.content, res.ui_type.map(|s| s.to_string()), true),
                    Err(e) => {
                        // Built-in miss → try the plugin router (built-in
                        // names are reserved and can't be shadowed).
                        if let Some(mgr) = &self.plugin_router {
                            match mgr.dispatch_tool_call(&call.name, args).await {
                                Ok(out) => (out, None, true),
                                Err(_) => (e.to_string(), None, false),
                            }
                        } else {
                            (e.to_string(), None, false)
                        }
                    }
                };

                // ---- Stage 9: after_tool hooks (may mutate output) ----
                self.hooks.run_after_tool(&call, &mut content).await;

                // Record the write post-dispatch so Undo sees every mutation.
                if is_write && ok {
                    if let Some(p) = write_path.as_deref() {
                        if let Ok(abs) = self.ctx.resolve(p) {
                            if let Ok(new_content) = std::fs::read_to_string(&abs) {
                                hunks.record_write(abs, old_content, new_content, "agent");
                            }
                        }
                    }
                }

                let _ = io.ui_tx.try_send(UiEvent::ToolCallFinished {
                    name: call.name.clone(),
                    ok,
                    content: content.clone(),
                    ui_type,
                });

                history.push(ChatMessage::tool_result(call.id.clone(), content));
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

        // Build the summarize request over the prefix.
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
    /// or the safety timeout fires (fail-closed: deny on timeout).
    async fn wait_for_decision(&self) -> bool {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            if let Some(v) = self.decision.lock().unwrap().take() {
                return v;
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
