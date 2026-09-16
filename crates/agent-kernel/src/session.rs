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
    decision_slot: Arc<std::sync::Mutex<Option<bool>>>,
    /// Forwards `UiCommand::Steer` texts into the engine's steer channel.
    steer_tx: mpsc::Sender<String>,
    /// Actor-owned command receiver — `run()` drains it.
    cmd_rx: mpsc::Receiver<UiCommand>,
    /// Workspace root (undo writes resolve against it).
    workspace_root: PathBuf,
    /// Stage 9: ordered hook chain (lifecycle interception).
    hooks: HookChain,
    /// Stage 10: memory store + post-turn distiller (background task).
    memory: Option<Arc<MemoryStore>>,
    distiller: Option<Arc<TurnDistiller>>,
}

impl SessionActor {
    /// Boot a session: scan workspace + git snapshot → render system prompt →
    /// return the actor plus the UI channel endpoints.
    pub fn spawn(cfg: SessionConfig) -> (Self, super::channels::UiChannels) {
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

        // Stage 10: memory store — `memory.db` at ~/.local/share/agent-rs/,
        // partitioned by `hash(canonical_root)`. The `{{MEMORY_BLOCK}}` is
        // refreshed per-turn (recall happens in `run_prompt`, not once at
        // spawn — the block must track the evolving store).
        let (memory, distiller, initial_memory_block) = {
            let db_dir = dirs_data().map(|d| d.join("agent-rs"));
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

        let history = vec![ChatMessage::system(system_prompt)];

        let registry = Arc::new(ToolRegistry::with_builtins());
        let ctx = Arc::new(ToolCtx::new(&cfg.workspace_root));
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
        let decision_slot = engine.decision_slot();

        // Actor keeps the real cmd_rx — `run()` drains it into `handle()`.
        // The `channels.cmd_rx` left for the caller is a dead end; the
        // frontend sends on `command_sender()`.
        let cmd_rx = std::mem::replace(&mut channels.cmd_rx, mpsc::channel(1).1);
        let cmd_tx = channels.cmd_tx.clone();

        // Engine gets a private steering channel — SessionActor forwards
        // `UiCommand::Steer` texts into it mid-turn (engine drains between
        // tool calls). Between turns a Steer is a Prompt.
        let (steer_tx, steer_rx) = mpsc::channel::<String>(32);
        let io = EngineIo {
            ui_tx: channels.event_tx.clone(),
            steer_rx,
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
                steer_tx,
                cmd_rx,
                workspace_root: cfg.workspace_root,
                hooks: HookChain::new(),
                memory,
                distiller,
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
    pub fn decision_writer(&self) -> Arc<std::sync::Mutex<Option<bool>>> {
        self.decision_slot.clone()
    }

    /// The mid-turn steering sender — `Steer` texts go here when a turn is
    /// already in flight (the engine drains between tool calls). Between
    /// turns a Steer is a Prompt — the bridge checks `state().is_active()`.
    pub fn steer_writer(&self) -> mpsc::Sender<String> {
        self.steer_tx.clone()
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
        self.state = AgentState::ScanningWorkspace;
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
                info!(tool_calls = outcome.tool_calls_run, turn, "turn finished");
                self.queue_distill(TurnRecord {
                    task: outcome_text,
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
                self.queue_distill(TurnRecord {
                    task: outcome_text,
                    outcome: "failed".into(),
                    files: self.hunks.files_in_turn(turn).iter()
                        .map(|p| p.to_string_lossy().into_owned()).collect(),
                    correction: None,
                    steered_with: None,
                });
            }
        }
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
        let plan = self.hunks.undo_plan(last);
        if plan.is_empty() {
            let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(
                "nothing to undo".into(),
            ));
            return;
        }
        let mut reverted = 0usize;
        for op in plan {
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
        let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(
            format!("reverted {reverted} file(s) from turn {last}"),
        ));
    }

    /// Execute a `ControlOp` from a slash command — session state changes
    /// with no LLM round-trip.
    async fn run_control(&mut self, op: ControlOp) {
        match op {
            ControlOp::ClearHistory => {
                self.history.truncate(1); // keep the system prompt
                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(
                    "history cleared".into()));
            }
            ControlOp::Compact => {
                // Force a compaction pass via the engine's path.
                let est = crate::compaction::estimate_tokens(&self.history);
                let window = 256_000usize; // engine's context_window is the real bound
                if crate::compaction::should_compact(est, window) {
                    let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(
                        "compacting…".into()));
                } else {
                    let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(
                        format!("history ~{est} tokens — below compact threshold")));
                }
            }
            ControlOp::UndoLastTurn => self.undo_last_turn().await,
            ControlOp::SetModel { provider, model } => {
                info!(%provider, %model, "slash hot-swap");
                self.engine.set_model(model.clone());
                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(
                    format!("model → {provider}/{model} (next turn)")));
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
                        let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(r));
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
                                let _ = self.io.ui_tx.try_send(UiEvent::SystemMessage(reason));
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
                self.engine.set_model(model);
                // provider swap lands via a rebuilt provider Arc — the caller
                // passes the new Arc through SessionConfig on rebuild; Stage 5
                // wires the UI dropdown to reconstruct the session.
            }
            UiCommand::Cancel => {
                // Cooperative cancel: in-flight streams finish their current
                // chunk then the turn unwinds. Full cancellation lands with
                // the steering stage; for now mark Finished and emit notice.
                self.state = AgentState::Finished;
                let _ = self
                    .io
                    .ui_tx
                    .try_send(UiEvent::SystemMessage("turn cancelled".into()));
            }
            UiCommand::ToolDecision { approved } => {
                // Direct write to the engine's shared decision slot — this
                // reaches the wait even when handle() is invoked mid-turn
                // via run()'s serial pump (the turn future polls the slot,
                // not this channel).
                *self.decision_slot.lock().unwrap() = Some(approved);
            }
            UiCommand::UndoLastTurn => {
                self.undo_last_turn().await;
            }
        }
    }
}

/// Data dir for `memory.db` — `~/.local/share` (XDG) or HOME fallback.
fn dirs_data() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
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
