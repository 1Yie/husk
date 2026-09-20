//! `SessionActor` — the orchestrator actor that owns history, state, and the
//! engine, communicating with the UI over `tokio::mpsc`.
//!
//! Owns: conversation history, `AgentState`, tool dispatch (via Engine),
//! and — in later stages — permission manager, compaction, hunk tracker.
//! This is the single writer of `history`: tools never mutate it.

use std::path::PathBuf;
use std::sync::Arc;

use agent_context::{
    git_snapshot, memory::TurnRecord, HunkTracker, MemoryStore, TrackingMode,
    TurnDistiller, WorkspaceScanner,
};
use agent_ipc::{AgentState, UiCommand, UiEvent};
use agent_llm::types::ChatMessage;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::channels::ui_channels;
use crate::commands::{CommandCtx, CommandRegistry, CommandResult, ControlOp};
use crate::engine::{Engine, EngineIo};
use crate::hooks::HookChain;
use crate::tools::{ToolCtx, ToolRegistry};

/// The rendered system prompt template — embedded at build time from the
/// skill's canonical source (single source of truth, never hand-edited here).
const SYSTEM_PROMPT_TEMPLATE: &str = include_str!("../assets/kernel-system-prompt.md");

/// SessionActor configuration for one workspace.
pub struct SessionConfig {
    pub workspace_root: PathBuf,
    pub provider: Arc<dyn agent_llm::LlmProvider>,
    pub model: String,
    pub temperature: f32,
    /// Permission mode label substituted into the prompt.
    pub permission_mode: String,
    /// Whether to seed the tracker with git-dirty files (`AllDirty` mode).
    pub track_dirty: bool,
    /// Initial thinking / reasoning effort level.
    pub thinking_level: Option<String>,
    /// Model-specific thinking level mapping.
    pub thinking_level_map: Option<std::collections::HashMap<String, Option<String>>>,
    /// Model's context window from config — `None` falls back to the
    /// engine's 256_000 default (compaction + usage % share the bound).
    pub context_window: Option<u64>,
    /// Declared input modalities (`ModelConfig.input`, e.g.
    /// `["text", "image"]`) — `"image"` gates real image parts on the wire.
    pub model_input: Vec<String>,
}

/// One live session: owns history + engine, consumes commands, emits events.
pub struct SessionActor {
    history: Vec<ChatMessage>,
    engine: Engine,
    state: AgentState,
    io: EngineIo,
    cmd_tx: mpsc::Sender<UiCommand>,
    /// Stage 6: write tracking for undo/rewind + files-changed list.
    hunks: HunkTracker,
    /// Engine's decision slot — `ToolDecision` writes here mid-turn.
    decision_slot: Arc<std::sync::Mutex<Option<(u64, bool)>>>,
    /// Engine's permissions slot — permission mode updates apply immediately mid-turn.
    permissions_slot: Arc<std::sync::RwLock<crate::permissions::PermissionGate>>,
    /// Engine's thinking-level slot — read by the manager to report the
    /// session's live level (defaults may differ once prefs change).
    thinking_slot: Arc<std::sync::RwLock<Option<String>>>,
    /// Forwards `UiCommand::Steer` texts into the engine's steer channel.
    steer_tx: mpsc::Sender<String>,
    /// Cooperative cancel flag shared with the engine — `UiCommand::Cancel`
    /// and `cancel_writer()` set it; the engine polls it between chunks and
    /// before each tool dispatch.
    cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Actor-owned command receiver — `run()` drains it.
    cmd_rx: mpsc::Receiver<UiCommand>,
    /// Workspace root (undo writes resolve against it).
    workspace_root: PathBuf,
    /// Stage 9: ordered hook chain (lifecycle interception).
    hooks: HookChain,
    /// Stage 10: memory store + post-turn distiller (background task).
    memory: Option<Arc<MemoryStore>>,
    distiller: Option<Arc<TurnDistiller>>,
    /// Stable session identity — sidebar ordering + persistence file name.
    session_id: i64,
    /// Per-workspace session store — snapshots history at turn boundaries.
    store: Option<Arc<crate::session_store::SessionStore>>,
    /// Last completed turn's `(prompt, completion)` tokens — persisted
    /// into `SessionMeta` so a reopened session's header meter shows real
    /// numbers before the next `Usage` event.
    last_usage: Option<(u32, u32)>,
}

impl SessionActor {
    /// Boot a session: scan workspace + git snapshot → render system prompt →
    /// return the actor plus the UI channel endpoints.
    pub fn spawn(cfg: SessionConfig) -> (Self, super::channels::UiChannels) {
        Self::spawn_inner(cfg, None, 0)
    }

    /// Fresh session with a caller-assigned id (the SessionManager allocates
    /// ids via `SessionStore::next_id` so persistence + sidebar agree).
    pub fn spawn_with_id(cfg: SessionConfig, session_id: i64) -> (Self, super::channels::UiChannels) {
        Self::spawn_inner(cfg, None, session_id)
    }

    /// Resume a persisted session — `history` is the prior `Vec<ChatMessage>`
    /// snapshot; the LLM context is exactly what the session left behind.
    /// `session_id` reuses the persisted id so the store/app keep tracking it.
    pub fn resume(
        cfg: SessionConfig,
        session_id: i64,
        history: Vec<ChatMessage>,
    ) -> (Self, super::channels::UiChannels) {
        // Sanitize persisted history (a snapshot poisoned by an old bug
        // replays into every later request and crashes devin's upstream as
        // `invalid_argument`):
        //  1. strip `[call: …]` text-protocol echoes from assistant content;
        //  2. drop assistant rows that are empty AND carry no tool_calls —
        //     a blank turn replays as a meaningless message;
        //  3. drop orphan tool rows — a `tool` message whose `tool_call_id`
        //     never appeared on an assistant's `tool_calls` replays as a
        //     `function_call_output` with no matching call → invalid_argument.
        let mut history = history;
        for m in &mut history {
            if m.role == agent_llm::types::Role::Assistant {
                if let Some(text) = &mut m.content {
                    *text = strip_call_echo(text);
                    if text.trim().is_empty() {
                        m.content = None;
                    }
                }
            }
        }
        let mut known_call_ids = std::collections::HashSet::new();
        history.retain(|m| {
            match m.role {
                agent_llm::types::Role::Assistant => {
                    for c in m.tool_calls.iter().flatten() {
                        known_call_ids.insert(c.id.clone());
                    }
                    // keep assistant if it has content OR tool_calls
                    m.content.is_some() || m.tool_calls.is_some()
                }
                agent_llm::types::Role::Tool => {
                    m.tool_call_id
                        .as_ref()
                        .map(|id| known_call_ids.contains(id))
                        .unwrap_or(false)
                }
                _ => true,
            }
        });
        Self::spawn_inner(cfg, Some(history), session_id)
    }

    /// Shared spawn path — `prior_history` replaces the fresh system-prompt
    /// history when resuming; `session_id` is 0 for a fresh session (the
    /// caller assigns a real id via the store).
    fn spawn_inner(
        cfg: SessionConfig,
        prior_history: Option<Vec<ChatMessage>>,
        session_id: i64,
    ) -> (Self, super::channels::UiChannels) {
        let mut channels = ui_channels();

        // ---- Stage 1 scan: workspace skeleton + git snapshot ----
        let workspace_tree = WorkspaceScanner::build_skeleton(&cfg.workspace_root)
            .map(|t| t.to_prompt_block())
            .unwrap_or_else(|e| format!("(workspace scan failed: {e})\n"));
        let git_status = git_snapshot(&cfg.workspace_root)
            .map(|s| s.to_prompt_block())
            .unwrap_or_else(|_| "(not a git repository)\n".to_string());

        // ---- Render the kernel system prompt ----
        let system_prompt = SYSTEM_PROMPT_TEMPLATE
            .replace("{{DATE}}", &chrono_lite_date())
            .replace("{{WORKSPACE_ROOT}}", &cfg.workspace_root.display().to_string())
            .replace("{{PERMISSION_MODE}}", &cfg.permission_mode)
            .replace("{{WORKSPACE_TREE}}", &workspace_tree)
            .replace("{{GIT_STATUS}}", &git_status);

        // Stage 10: memory store — `memory.db` at ~/.local/share/husk/,
        // partitioned by `hash(canonical_root)`. The `{{MEMORY_BLOCK}}` is
        // refreshed per-turn (recall happens in `run_prompt`, not once at
        // spawn — the block must track the evolving store).
        let (memory, distiller, initial_memory_block) = {
            let db_dir = crate::session_store::app_data_dir();
            let store = db_dir.and_then(|d| {
                let _ = std::fs::create_dir_all(&d);
                MemoryStore::open(&d.join("memory.db"), &cfg.workspace_root).ok()
            });
            match store {
                Some(s) => {
                    let s = Arc::new(s);
                    let block = s.memory_block("workspace").unwrap_or_default();
                    (Some(s.clone()), Some(Arc::new(TurnDistiller::new(s))), block)
                }
                None => (None, None, "(no memory)".to_string()),
            }
        };
        let system_prompt = system_prompt
            .replace("{{MEMORY_BLOCK}}", &initial_memory_block);

        // Fresh session → just the rendered system prompt; resume → the
        // persisted history (its system prompt was rendered at that session's
        // spawn — memory/workspace baked in for that turn).
        let history = prior_history.unwrap_or_else(|| vec![ChatMessage::system(system_prompt)]);

        // Session persistence — per-workspace store; the actor snapshots
        // `history` at turn boundaries so a crashed/killed app resumes mid-
        // conversation from the last completed turn.
        let store = crate::session_store::SessionStore::open(&cfg.workspace_root)
            .ok()
            .map(Arc::new);

        let registry = Arc::new(ToolRegistry::with_builtins());
        let ctx = Arc::new(
            ToolCtx::new(&cfg.workspace_root).with_session(session_id, store.clone()),
        );
        let mut engine = Engine::new(
            cfg.provider.clone(),
            registry,
            ctx,
            cfg.model.clone(),
            cfg.temperature,
        );
        engine.set_permissions(crate::permissions::PermissionGate::from_mode_str(
            &cfg.permission_mode,
        ));
        engine.set_thinking_level(cfg.thinking_level.clone());
        engine.set_thinking_level_map(cfg.thinking_level_map.clone());
        engine.set_context_window(cfg.context_window.unwrap_or(256_000) as usize);
        engine.set_model_input(cfg.model_input.clone());
        let decision_slot = engine.decision_slot();
        let permissions_slot = engine.permissions_writer();
        let thinking_slot = engine.thinking_level_shared();

        // Actor keeps the real cmd_rx — `run()` drains it into `handle()`.
        // The `channels.cmd_rx` left for the caller is a dead end; the
        // frontend sends on `command_sender()`.
        let cmd_rx = std::mem::replace(&mut channels.cmd_rx, mpsc::channel(1).1);
        let cmd_tx = channels.cmd_tx.clone();

        // Engine gets a private steering channel — SessionActor forwards
        // `UiCommand::Steer` texts into it mid-turn (engine drains between
        // tool calls). Between turns a Steer is a Prompt.
        let (steer_tx, steer_rx) = mpsc::channel::<String>(32);
        // P0-C4: cooperative cancel flag shared with the engine — the UI
        // writes it via `cancel_writer()` (bypassing the command pump so a
        // Cancel lands mid-turn); the engine checks it between chunks and
        // before each tool dispatch.
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let io = EngineIo {
            ui_tx: channels.event_tx.clone(),
            steer_rx,
            cancel: cancel.clone(),
        };

        // Stage 6: hunk tracker — `AllDirty` seeds with git-dirty files so a
        // dirty-tree warning can precede the first prompt.
        let mut hunks = HunkTracker::new(if cfg.track_dirty {
            TrackingMode::AllDirty
        } else {
            TrackingMode::AgentOnly
        });
        if cfg.track_dirty {
            if let Ok(snap) = git_snapshot(&cfg.workspace_root) {
                // status lines are porcelain XY-prefixed (` M a.rs`, `?? b.rs`)
                for line in &snap.status {
                    let p = line.get(3..).map(str::trim).unwrap_or("");
                    if !p.is_empty() {
                        hunks.mark_dirty_at_start(PathBuf::from(p));
                    }
                }
            }
        }

        (
            Self {
                history,
                engine,
                state: AgentState::Idle,
                io,
                cmd_tx,
                hunks,
                decision_slot,
                permissions_slot,
                thinking_slot,
                steer_tx,
                cancel,
                cmd_rx,
                workspace_root: cfg.workspace_root,
                hooks: HookChain::new(),
                memory,
                distiller,
                session_id,
                store,
                last_usage: None,
            },
            channels,
        )
    }

    /// The actor's command pump — `run()` consumes `UiCommand`s until the
    /// frontend's sender drops. Turns run in-line (a `Prompt`'s `run_turn`
    /// `await`s to completion inside `handle`).
    ///
    /// Mid-turn `ToolDecision`/`Cancel`/`Steer` do **not** flow through this
    /// pump — they'd block behind the running turn. Instead:
    ///   - `ToolDecision` → the shared `decision_slot` (engine polls it in
    ///     `wait_for_decision`); the bridge writes it directly.
    ///   - `Cancel`      → `EngineIo::cancel` flag (checked between chunks).
    ///   - `Steer`       → `steer_tx` → engine's `steer_rx` drain.
    /// `handle()` stays for tests that drive the actor directly.
    pub async fn run(&mut self) {
        while let Some(cmd) = self.cmd_rx.recv().await {
            self.handle(cmd).await;
        }
    }

    /// The frontend's command sender — Stage 5 bridge clones this.
    /// `Prompt`/`Steer`/`SetModel`/`UndoLastTurn` go here.
    pub fn command_sender(&self) -> mpsc::Sender<UiCommand> {
        self.cmd_tx.clone()
    }

    /// The shared tool-decision slot — the bridge writes `ToolDecision`
    /// **here directly**, bypassing the command channel so a mid-turn
    /// approval isn't queued behind the running `run_turn`.
    pub fn decision_writer(&self) -> Arc<std::sync::Mutex<Option<(u64, bool)>>> {
        self.decision_slot.clone()
    }

    /// The shared permission gate slot — updates apply immediately mid-turn.
    pub fn permissions_writer(&self) -> Arc<std::sync::RwLock<crate::permissions::PermissionGate>> {
        self.permissions_slot.clone()
    }

    /// The shared thinking-level slot — read-only mirror of the engine's
    /// level so the manager can report the session's actual value.
    pub fn thinking_writer(&self) -> Arc<std::sync::RwLock<Option<String>>> {
        self.thinking_slot.clone()
    }

    /// The mid-turn steering sender — `Steer` texts go here when a turn is
    /// already in flight (the engine drains between tool calls). Between
    /// turns a Steer is a Prompt — the bridge checks `state().is_active()`.
    pub fn steer_writer(&self) -> mpsc::Sender<String> {
        self.steer_tx.clone()
    }

    /// The shared cancel flag — the UI writes `true` here **directly** to
    /// abort the in-flight turn, bypassing the command pump (a `UiCommand::
    /// Cancel` queued behind `run_turn` would never be seen in time).
    /// `run_prompt` clears it at turn start.
    pub fn cancel_writer(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.cancel.clone()
    }

    /// Current state-machine position.
    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Read-only view of history length (for compaction pressure checks).
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Shared Prompt/Steer-between-turns path — extracted so `Steer` doesn't
    /// need a recursive `handle` call (async recursion requires boxing).
    async fn run_prompt(&mut self, text: String) {
        info!("user prompt ({} chars)", text.len());
        // Clear the cancel flag so a stale Cancel from a previous turn can't
        // abort this one before it starts.
        self.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        self.state = AgentState::ScanningWorkspace;
        // Echo the prompt so the UI stream renders a user block.
        let _ = self.io.ui_tx.try_send(UiEvent::UserPrompt(text.clone()));
        let turn = self.hunks.begin_turn();

        // Stage 10: refresh the `<memory>` block before the turn — recall
        // top-k facts + recent episodes for this prompt. The block lives in
        // the system message (history[0]); swap its placeholder region.
        if let Some(m) = &self.memory {
            if let Ok(block) = m.memory_block(&text) {
                if !block.trim().is_empty() {
                    if let Some(sys) = self.history.get_mut(0) {
                        if let Some(c) = &sys.content {
                            let new = c.replace("(no memory)", &block);
                            sys.content = Some(new);
                        }
                    }
                }
            }
        }

        let outcome_text = text.clone();
        match self.engine.run_turn(&mut self.io, &mut self.history, text, &mut self.hunks).await {
            Ok(outcome) => {
                self.state = AgentState::Finished;
                self.last_usage = outcome.usage;
                let _ = self.io.ui_tx.try_send(UiEvent::StateChanged(AgentState::Finished));
                info!(tool_calls = outcome.tool_calls_run, turn, "turn finished");
                self.queue_distill(TurnRecord {
                    task: outcome_text.clone(),
                    outcome: "success".into(),
                    files: self.hunks.files_in_turn(turn).iter()
                        .map(|p| p.to_string_lossy().into_owned()).collect(),
                    correction: None,
                    steered_with: None,
                });
            }
            Err(e) => {
                self.state = AgentState::Failed(e.clone());
                warn!("turn failed: {e}");
                // `run_turn` emits its own display line before every Err —
                // a SystemMessage for cancel, an Error for transport and
                // in-stream failures. Re-emitting here prints the same
                // reminder twice (the cancelled-turn double-line bug).
                let _ = self.io.ui_tx.try_send(UiEvent::StateChanged(AgentState::Failed(e.clone())));
                self.queue_distill(TurnRecord {
                    task: outcome_text.clone(),
                    outcome: "failed".into(),
                    files: self.hunks.files_in_turn(turn).iter()
                        .map(|p| p.to_string_lossy().into_owned()).collect(),
                    correction: None,
                    steered_with: None,
                });
            }
        }
        // Persist at the turn boundary — the store's last JSONL line is the
        // resumable state; sidebar meta updates so the session list reflects
        // the latest preview even if this session isn't the visible one.
        self.persist_turn(&outcome_text);
    }

    /// Snapshot `history` to the session store + update sidebar metadata.
    /// `title` derives from the first user prompt; `preview` from the last
    /// agent reply (or "running…" when mid-turn, per product spec).
    fn persist_turn(&mut self, last_task: &str) {
        let Some(store) = &self.store else { return };
        let id = self.session_id;
        // Sanitize before persisting — strip any `[call:…]` text-protocol
        // echo a provider leaked into assistant content (the streaming
        // CallStripper catches it live, but this is the last-line defense so
        // a poisoned snapshot can't re-teach the model on resume).
        let sanitized: Vec<ChatMessage> = self.history.iter().map(|m| {
            if m.role == agent_llm::types::Role::Assistant {
                let mut m = m.clone();
                if let Some(c) = &m.content {
                    m.content = Some(strip_call_echo(c));
                }
                m
            } else {
                m.clone()
            }
        }).collect();
        let _ = store.snapshot(id, &sanitized);

        // Title = first user prompt (truncated); preview = last agent text.
        let title = self.history.iter()
            .find(|m| m.role == agent_llm::types::Role::User)
            .and_then(|m| m.content.clone())
            .map(|c| c.chars().take(40).collect())
            .unwrap_or_else(|| last_task.chars().take(40).collect());
        let running = self.state.is_active();
        let preview = if running {
            "⟳ running…".to_string()
        } else {
            self.history.iter().rev()
                .find(|m| m.role == agent_llm::types::Role::Assistant)
                .and_then(|m| m.content.clone())
                .map(|c| c.chars().take(60).collect())
                .unwrap_or_else(|| "(no reply)".into())
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Usage rides along in the sidebar meta: fresh value when this turn
        // produced one, otherwise the previously persisted value survives
        // (a failed turn must not erase the last good reading; a resumed
        // actor's `last_usage` starts empty until its first turn lands).
        let usage = self
            .last_usage
            .map(|(prompt, completion)| crate::session_store::SessionUsage {
                prompt,
                completion,
                context_window: self.engine.context_window() as u32,
            })
            .or_else(|| {
                store
                    .list()
                    .into_iter()
                    .find(|m| m.id == id)
                    .and_then(|m| m.usage)
            });
        let _ = store.upsert_meta(crate::session_store::SessionMeta {
            id,
            title,
            preview,
            updated_at: now,
            usage,
        });
    }

    /// Session identity — the sidebar/store key.
    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    /// Read-only history snapshot (persistence + multi-session handoff).
    pub fn history(&self) -> &[ChatMessage] {
        &self.history
    }

    /// Stage 10: hand the turn's record to the background distiller — never
    /// blocks the turn's completion path.
    fn queue_distill(&mut self, record: TurnRecord) {
        if let Some(d) = &self.distiller {
            d.spawn_distill(record);
        }
    }

    /// Reverse-apply the last turn's writes via the HunkTracker — restores
    /// each touched file to its pre-turn state (or deletes it if created).
    async fn undo_last_turn(&mut self) {
        let last = self.hunks.current_turn();
        // P0-C2: undo_plan now refuses when a target file was modified
        // externally after the agent's last write — undoing would silently
        // clobber the user's own edits. Fall back to partial undo so the
        // rest still reverts, and surface which files were skipped.
        let (ops, skipped) = match self.hunks.undo_plan(last) {
            Ok(ops) => (ops, Vec::new()),
            Err(_) => self.hunks.undo_plan_partial(last),
        };
        if ops.is_empty() && skipped.is_empty() {
            let line = "没有可回滚的修改".to_string();
            let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
            self.history.push(ChatMessage::notice(line));
            return;
        }
        for path in &skipped {
            let line = format!(
                "已跳过 `{}` — 在 agent 写入后被外部修改（回滚会覆盖你的改动）",
                path.display()
            );
            let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
            self.history.push(ChatMessage::notice(line));
        }
        let mut reverted = 0usize;
        for op in ops {
            // Safety: never undo outside the workspace — the tracker records
            // resolved paths, but guard here so a bad origin can't delete
            // arbitrary files.
            if !op.path.starts_with(&self.workspace_root) {
                warn!(path = %op.path.display(), "undo skipped: outside workspace");
                continue;
            }
            match &op.restore_to {
                Some(content) => {
                    if tokio::fs::write(&op.path, content).await.is_ok() {
                        reverted += 1;
                    }
                }
                None => {
                    if tokio::fs::remove_file(&op.path).await.is_ok() {
                        reverted += 1;
                    }
                }
            }
        }
        let line = format!("已回滚第 {last} 轮修改的 {reverted} 个文件");
        let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
        self.history.push(ChatMessage::notice(line));
    }

    /// Execute a `ControlOp` from a slash command — session state changes
    /// with no LLM round-trip.
    async fn run_control(&mut self, op: ControlOp) {
        match op {
            ControlOp::ClearHistory => {
                self.history.truncate(1); // keep the system prompt
                let line = "会话历史已清空".to_string();
                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
            ControlOp::Compact => {
                // Force a compaction pass via the engine's path.
                let est = crate::compaction::estimate_tokens(&self.history);
                let window = 256_000usize; // engine's context_window is the real bound
                if crate::compaction::should_compact(est, window) {
                    let line = "正在压缩历史上下文…".to_string();
                    let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                    self.history.push(ChatMessage::notice(line));
                } else {
                    let line = format!("历史约 {est} tokens — 未达压缩阈值");
                    let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                    self.history.push(ChatMessage::notice(line));
                }
            }
            ControlOp::UndoLastTurn => self.undo_last_turn().await,
            ControlOp::SetModel { provider, model } => {
                info!(%provider, %model, "slash hot-swap");
                if let Ok(cfg) = agent_llm::AppConfig::load(None) {
                    if let Some(pcfg) = cfg.providers.get(&provider) {
                        if let Ok(p) = agent_llm::ProviderFactory::build(pcfg) {
                            self.engine.set_provider(p);
                        }
                        if let Some(mentry) = pcfg.find_model(&model) {
                            if let Some(d) = mentry.detailed() {
                                self.engine.set_thinking_level_map(d.thinking_level_map.clone());
                                self.engine.set_context_window(
                                    d.context_window.unwrap_or(256_000) as usize,
                                );
                                self.engine.set_model_input(d.input.clone());
                            } else {
                                self.engine.set_thinking_level_map(None);
                                self.engine.set_context_window(256_000);
                                self.engine.set_model_input(Vec::new());
                            }
                        } else {
                            self.engine.set_thinking_level_map(None);
                            self.engine.set_context_window(256_000);
                            self.engine.set_model_input(Vec::new());
                        }
                    }
                }
                self.engine.set_model(model.clone());
                let line = format!("已切换模型至: {model} ({provider}) — 下一轮生效");
                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
        }
    }

    /// Consume a `UiCommand`. `Prompt` runs a turn to completion; `Steer`
    /// lands mid-turn via the engine's drain; the rest are no-ops until their
    /// owning stages land.
    pub async fn handle(&mut self, cmd: UiCommand) {
        match cmd {
            UiCommand::Prompt { text } => {
                // Stage 9: `/x` slash commands intercept before the ReAct
                // loop — zero tokens. Unknown `/x` falls through as a prompt.
                let workspace_root = self.workspace_root.clone();
                let cmd_result = {
                    let mut ctx = CommandCtx {
                        history: &mut self.history,
                        permission_mode: "default",
                        workspace_root: &workspace_root,
                        ui_tx: &self.io.ui_tx,
                    };
                    CommandRegistry::try_run(&text, &mut ctx).await
                }; // ctx dropped here — the history/ui_tx borrows end
                match cmd_result {
                    Some(CommandResult::Reply(r)) => {
                        let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(r.clone()));
                        self.history.push(ChatMessage::notice(r));
                    }
                    Some(CommandResult::Control(op)) => {
                        self.run_control(op).await;
                    }
                    Some(CommandResult::FeedToAgent(prompt)) => {
                        self.run_prompt(prompt).await;
                    }
                    None => {
                        // Hook chain: on_user_input may block/rewrite.
                        match self.hooks.run_on_user_input(&text).await {
                            crate::hooks::HookAction::BlockTurn(reason) => {
                                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(reason.clone()));
                                self.history.push(ChatMessage::notice(reason));
                            }
                            crate::hooks::HookAction::InjectSystemNote(note) => {
                                self.history.push(ChatMessage::system(note));
                                self.run_prompt(text).await;
                            }
                            crate::hooks::HookAction::MutateMessages(msgs) => {
                                self.history = msgs;
                                self.run_prompt(text).await;
                            }
                            crate::hooks::HookAction::Continue => {
                                self.run_prompt(text).await;
                            }
                        }
                    }
                }
            }
            UiCommand::Steer { text } => {
                info!(len = text.len(), "steer command");
                if self.state.is_active() {
                    // Mid-turn: forward into the engine's steer channel —
                    // drain_steering picks it up between tool calls.
                    let _ = self.steer_tx.try_send(text);
                } else {
                    // Between turns a Steer is a fresh Prompt.
                    self.run_prompt(text).await;
                }
            }
            UiCommand::SetModel { provider, model } => {
                info!(%provider, %model, "hot-swap requested");
                if let Ok(cfg) = agent_llm::AppConfig::load(None) {
                    if let Some(pcfg) = cfg.providers.get(&provider) {
                        if let Ok(p) = agent_llm::ProviderFactory::build(pcfg) {
                            self.engine.set_provider(p);
                        }
                        if let Some(mentry) = pcfg.find_model(&model) {
                            if let Some(d) = mentry.detailed() {
                                self.engine.set_thinking_level_map(d.thinking_level_map.clone());
                                self.engine.set_context_window(
                                    d.context_window.unwrap_or(256_000) as usize,
                                );
                                self.engine.set_model_input(d.input.clone());
                            } else {
                                self.engine.set_thinking_level_map(None);
                                self.engine.set_context_window(256_000);
                                self.engine.set_model_input(Vec::new());
                            }
                        } else {
                            self.engine.set_thinking_level_map(None);
                            self.engine.set_context_window(256_000);
                            self.engine.set_model_input(Vec::new());
                        }
                    }
                }
                self.engine.set_model(model.clone());
                let line = format!("已切换模型至: {model} ({provider})");
                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
            UiCommand::SetThinkingLevel { level } => {
                info!(%level, "thinking level change requested");
                self.engine.set_thinking_level(if level.is_empty() { None } else { Some(level.clone()) });
                let line = format!("已设置思考推理强度: {level}");
                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(line.clone()));
                self.history.push(ChatMessage::notice(line));
            }
            UiCommand::SetPermissionMode { mode } => {
                info!(%mode, "permission mode switch");
                self.engine.set_permissions(
                    crate::permissions::PermissionGate::from_mode_str(&mode),
                );
                let _ = self
                    .io
                    .ui_tx
                    .try_send(UiEvent::SystemMessage(format!("权限模式已切换为: {mode}")));
                self.history.push(ChatMessage::notice(format!("权限模式已切换为: {mode}")));
            }
            UiCommand::Cancel => {
                // P0-C4: real cooperative cancel — set the shared flag the
                // engine polls between chunks and before each tool dispatch.
                // The command-pump path covers the between-turns case; the
                // mid-turn path is the `cancel_writer()` the bridge writes
                // directly so it isn't queued behind `run_turn`.
                self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                self.state = AgentState::Finished;
                let _ = self
                    .io
                    .ui_tx
                    .try_send(UiEvent::SystemMessage("已被用户中断".into()));
                self.history.push(ChatMessage::notice("已被用户中断"));
            }
            UiCommand::ToolDecision { request_id, approved } => {
                // Direct write to the engine's shared decision slot — this
                // reaches the wait even when handle() is invoked mid-turn
                // via run()'s serial pump (the turn future polls the slot,
                // not this channel). The request_id correlates the verdict
                // to a specific ApprovalRequested (P1-a).
                *self.decision_slot.lock().unwrap() = Some((request_id, approved));
            }
            UiCommand::UndoLastTurn => {
                self.undo_last_turn().await;
            }
        }
    }
}

/// `chrono`-free date stamp for the prompt — good enough for "today" context
/// without pulling a date crate into the kernel's dep graph.
fn chrono_lite_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // days since epoch → civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Remove `[call: name(args)]` / `[call: ()]` text-protocol tool-call echoes
/// from a whole assistant message (the non-streaming form of the adapter's
/// CallStripper — applied to persisted history on resume so a poisoned
/// snapshot can't re-teach the model). Drops each `[call:…]` run plus one
/// leading/trailing newline so no blank-line artifact remains.
fn strip_call_echo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[call:") {
        // Emit text before the call, trimming a directly-preceding newline.
        let before = &rest[..start];
        out.push_str(before.strip_suffix('\n').unwrap_or(before));
        // Skip the `[call:…]` run — to the closing `]` (or end of input).
        let after = &rest[start..];
        let end = after.find(']').map(|i| i + 1).unwrap_or(after.len());
        rest = &after[end..];
        // Swallow one newline right after the call line.
        if let Some(stripped) = rest.strip_prefix('\n') {
            rest = stripped;
        }
    }
    out.push_str(rest);
    out
}
