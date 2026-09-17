//! `bridge` — the only place kernel events become Slint property updates.
//!
//! Binding rules (slint-ui-contract.md):
//! - All model mutation goes through `slint::invoke_from_event_loop` — never
//!   touch Slint objects off the UI thread.
//! - `messages`/`active_steps` are `Rc<VecModel<T>>` owned HERE — the Slint
//!   `ModelRc` is set once at boot; we mutate through our own `Rc` handle so
//!   `set_row_data` applies in place (3-step streaming lifecycle).

use std::rc::Rc;
use std::sync::Arc;

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_llm::{AppConfig, ProviderFactory};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{
    ActionStepData, Bridge, CodexDesktop, DiffLineData, PendingApprovalData,
    SessionMessageData,
};

/// Owned model handles — the bridge mutates these, Slint reads them.
struct UiModels {
    messages: Rc<VecModel<SessionMessageData>>,
    steps: Rc<VecModel<ActionStepData>>,
    pending_diff: Rc<VecModel<DiffLineData>>,
    changed_files: Rc<VecModel<slint::SharedString>>,
}

/// Resolve the active provider from `config.toml` → `(provider, model,
/// label)`. Order: `active_provider` → first `available` provider → `Mock`.
fn resolve_provider(
    cfg: &AppConfig,
) -> (Arc<dyn agent_llm::LlmProvider>, String, String) {
    // Try the configured active provider first.
    if let Some(name) = &cfg.active_provider {
        if let Some(pcfg) = cfg.providers.get(name) {
            if let Ok(p) = ProviderFactory::build(pcfg) {
                let model = cfg
                    .active_model
                    .clone()
                    .or_else(|| pcfg.default_model.clone())
                    .unwrap_or_else(|| "default".into());
                return (p, model, name.clone());
            }
        }
    }
    // Then any available provider in declaration order.
    for (name, pcfg) in &cfg.providers {
        if let Ok(p) = ProviderFactory::build(pcfg) {
            let model = cfg
                .active_model
                .clone()
                .or_else(|| pcfg.default_model.clone())
                .unwrap_or_else(|| "default".into());
            return (p, model, name.clone());
        }
    }
    // Nothing configured/available → Mock so the UI still works.
    (
        Arc::new(agent_llm::adapters::MockProvider::new()),
        "mock".into(),
        "mock (no provider configured)".into(),
    )
}

/// Attach the kernel behind `--live`. Provider comes from `config.toml`
/// (`env:`/`keyring:` resolved); Mock is the no-config fallback.
pub fn wire_kernel(app: &CodexDesktop) {
    let app_weak = app.as_weak();
    let bridge = app.global::<Bridge>();

    // Owned models — installed on the global once, mutated through the Rc.
    let models = UiModels {
        messages: Rc::new(VecModel::from(Vec::<SessionMessageData>::new())),
        steps: Rc::new(VecModel::from(Vec::<ActionStepData>::new())),
        pending_diff: Rc::new(VecModel::from(Vec::<DiffLineData>::new())),
        changed_files: Rc::new(VecModel::from(Vec::<slint::SharedString>::new())),
    };
    bridge.set_messages(ModelRc::from(models.messages.clone()));
    bridge.set_active_steps(ModelRc::from(models.steps.clone()));
    bridge.set_pending_diff(ModelRc::from(models.pending_diff.clone()));
    bridge.set_changed_files(ModelRc::from(models.changed_files.clone()));

    // Real session list — a single live row for the current kernel session.
    // `new_session` resets the stream (fresh SessionActor); `select_session`
    // is a no-op until multi-session persistence lands.
    let sessions = Rc::new(VecModel::from(vec![crate::SessionData {
        id: 0,
        title: "current session".into(),
        preview: "live kernel".into(),
        active: true,
        timestamp: "now".into(),
    }]));
    bridge.set_sessions(ModelRc::from(sessions));
    bridge.on_new_session(|| {
        // A fresh SessionActor would spawn here — for now, log; full reset
        // (clear models + respawn actor) lands with multi-session support.
        eprintln!("[live] new_session — resets are not yet wired to a fresh actor");
    });
    bridge.on_select_session(|_id| {
        // Single live session — nothing to switch to yet.
    });

    // Sandbox status chip — `id() == "none"` is the loud-unsandboxed state.
    {
        let (_b, loud) = agent_sandbox::detect_backend();
        bridge.set_sandbox_unsafe(loud);
    }

    // ---- Stage 5.5: real provider from config.toml ----
    // `~/.config/agent-rs/config.toml` resolves `env:`/`keyring:` indirection
    // into a live provider; a missing/unsolvable key falls back to the first
    // `available` provider, and none → `Mock` so the app still boots.
    let cfg = AppConfig::load(None).unwrap_or_default();
    let (provider, model, provider_label) = resolve_provider(&cfg);

    // Surface the resolved provider + model on the stats bar.
    {
        let mut s = bridge.get_stats();
        s.active_provider = provider_label.clone().into();
        s.active_model = model.clone().into();
        bridge.set_stats(s);
    }

    let (mut actor, channels) = SessionActor::spawn(SessionConfig {
        workspace_root: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        provider,
        model,
        temperature: 1.0,
        permission_mode: "default".into(),
        track_dirty: true,
    });
    let cmd_tx = actor.command_sender();
    let decision_writer = actor.decision_writer();
    let steer_writer = actor.steer_writer();

    // ---- UI → kernel callbacks ----
    // Prompt goes through the command channel (serialized by actor.run()).
    // Steer/ToolDecision bypass it — they'd queue behind the running turn's
    // `await`ed run_prompt and never reach the engine in time.
    {
        let tx = cmd_tx.clone();
        bridge.on_submit_prompt(move |text| {
            let _ = tx.try_send(UiCommand::Prompt { text: text.to_string() });
        });
    }
    {
        let cmd = cmd_tx.clone();
        bridge.on_steer(move |text| {
            let t = text.to_string();
            // Engine drains steer between tool calls; when idle the actor
            // treats a Steer as a Prompt via the command channel.
            if steer_writer.try_send(t.clone()).is_err() {
                let _ = cmd.try_send(UiCommand::Steer { text: t });
            }
        });
    }
    {
        let tx = cmd_tx.clone();
        bridge.on_cancel_turn(move || {
            let _ = tx.try_send(UiCommand::Cancel);
        });
    }
    {
        let w = decision_writer.clone();
        bridge.on_approve_tool(move |_id, _remember| {
            *w.lock().unwrap() = Some(true);
        });
        bridge.on_deny_tool(move |_id, _reason| {
            *decision_writer.lock().unwrap() = Some(false);
        });
    }

    // ---- actor run() pump on a background task ----
    // `wire_kernel` runs on the Slint UI thread — no tokio reactor here, so
    // spawn the actor on a dedicated runtime thread. Slint's `spawn_local`
    // (below) drives the UI-side event pump without needing tokio.
    std::thread::Builder::new()
        .name("kernel-rt".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                actor.run().await;
            });
        })
        .expect("spawn kernel runtime thread");

    // ---- kernel → UI pump ----
    // `event_rx` is a tokio mpsc — it must be `.await`ed inside the tokio
    // reactor (the kernel-rt thread above), not Slint's executor. Forward
    // each event over a plain `std::sync::mpsc` and drain it on the UI
    // thread with a repeating timer — the documented Slint pump pattern.
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();
    {
        let mut rx = channels.event_rx;
        // Reuse the kernel-rt runtime handle for the forwarder.
        std::thread::Builder::new()
            .name("ui-forward".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio rt");
                rt.block_on(async move {
                    while let Some(ev) = rx.recv().await {
                        if ui_tx.send(ev).is_err() {
                            break; // UI gone
                        }
                    }
                });
            })
            .expect("spawn ui-forward");
    }
    // Drain on the UI thread at ~60 Hz — cheap and keeps `models` Rc-bound.
    {
        let weak = app_weak.clone();
        let models = std::rc::Rc::new(models);
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                while let Ok(ev) = ui_rx.try_recv() {
                    apply_event(&weak, ev, &models);
                }
            },
        );
        // Keep the timer alive for the window's lifetime.
        std::mem::forget(timer);
    }
}

/// Map one `UiEvent` onto Slint — runs on the UI thread via `spawn_local`,
/// so `invoke_from_event_loop` isn't needed (we're already on it).
fn apply_event(weak: &slint::Weak<CodexDesktop>, ev: UiEvent, m: &UiModels) {
    let Some(app) = weak.upgrade() else { return };
    let b = app.global::<Bridge>();
    eprintln!("[ui-ev] {ev:?}"); // session trace — remove before release

    match ev {
        UiEvent::StateChanged(s) => {
            let mut stats = b.get_stats();
            stats.agent_state = format!("{s:?}").into();
            b.set_stats(stats);
            b.set_is_active(s.is_active());
        }
        UiEvent::UserPrompt(text) => {
            m.messages.push(SessionMessageData {
                id: m.messages.row_count() as i32,
                role: "user".into(),
                text: text.into(),
                reasoning: "".into(),
                has_diff: false,
                streaming: false,
            });
        }
        UiEvent::TextDelta(t) => append_last(m, &t, false),
        UiEvent::ReasoningDelta(t) => append_last(m, &t, true),
        UiEvent::AssistantMessage(_) => {
            // finalize the streaming row
            let n = m.messages.row_count();
            if n > 0 {
                let mut row = m.messages.row_data(n - 1).unwrap();
                row.streaming = false;
                m.messages.set_row_data(n - 1, row);
            }
        }
        UiEvent::ToolCallStarted { name } => {
            m.steps.push(ActionStepData {
                id: m.steps.row_count() as i32,
                name: name.into(),
                state: "running".into(),
                detail: "".into(),
                expandable: true,
            });
        }
        UiEvent::ToolCallFinished { name, ok, content, .. } => {
            for i in (0..m.steps.row_count()).rev() {
                let mut row = m.steps.row_data(i).unwrap();
                if row.name.as_str() == name.as_str() && row.state == "running" {
                    row.state = if ok { "success".into() } else { "error".into() };
                    row.detail = content.chars().take(60).collect::<String>().into();
                    m.steps.set_row_data(i, row);
                    break;
                }
            }
        }
        UiEvent::ApprovalRequested { tool_name, diff, fuzzy } => {
            b.set_pending(PendingApprovalData {
                step_id: 0,
                tool_name: tool_name.clone().into(),
                title: format!("Approve {tool_name}").into(),
                command: "".into(),
                risk: "normal".into(),
                audit_reason: "".into(),
                diff_text: diff.clone().into(),
                fuzzy,
            });
            m.pending_diff.set_vec(parse_unified_diff(&diff));
        }
        UiEvent::Usage { prompt_tokens, completion_tokens } => {
            let mut s = b.get_stats();
            s.tokens_used = (prompt_tokens + completion_tokens) as i32;
            b.set_stats(s);
        }
        UiEvent::SystemMessage(msg) | UiEvent::Error(msg) => {
            m.messages.push(SessionMessageData {
                id: m.messages.row_count() as i32,
                role: "system".into(),
                text: msg.into(),
                reasoning: "".into(),
                has_diff: false,
                streaming: false,
            });
        }
    }
}

/// Append a delta to the *streaming agent row* — the last row only if it's
/// an agent row still streaming; otherwise push a fresh agent row. This is
/// what kept a user prompt from getting the reply glued onto it.
fn append_last(m: &UiModels, delta: &str, reasoning: bool) {
    let n = m.messages.row_count();
    let target = if n > 0 {
        let row = m.messages.row_data(n - 1).unwrap();
        if row.role.as_str() == "agent" && row.streaming {
            Some(n - 1)
        } else {
            None
        }
    } else {
        None
    };
    let i = match target {
        Some(i) => i,
        None => {
            m.messages.push(SessionMessageData {
                id: m.messages.row_count() as i32,
                role: "agent".into(),
                text: "".into(),
                reasoning: "".into(),
                has_diff: false,
                streaming: true,
            });
            m.messages.row_count() - 1
        }
    };
    let mut row = m.messages.row_data(i).unwrap();
    if reasoning {
        row.reasoning = format!("{}{}", row.reasoning, delta).into();
        row.streaming = true;
    } else {
        row.text = format!("{}{}", row.text, delta).into();
        row.streaming = true;
    }
    m.messages.set_row_data(i, row);
}

/// Split a unified diff into `DiffLineData` rows for the stage.
fn parse_unified_diff(diff: &str) -> Vec<DiffLineData> {
    let mut out = Vec::new();
    let (mut old_ln, mut new_ln) = (0i32, 0i32);
    for line in diff.lines() {
        if line.starts_with("@@") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                old_ln = parts[1].trim_start_matches('-').split(',').next()
                    .and_then(|s| s.parse().ok()).unwrap_or(0);
                new_ln = parts[2].trim_start_matches('+').split(',').next()
                    .and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            continue;
        }
        if line.starts_with("---") || line.starts_with("+++") || line.starts_with("diff ") {
            continue;
        }
        let (ty, o, n) = if line.starts_with('+') {
            let v = ("add", -1, new_ln); new_ln += 1; v
        } else if line.starts_with('-') {
            let v = ("delete", old_ln, -1); old_ln += 1; v
        } else {
            let v = ("context", old_ln, new_ln); old_ln += 1; new_ln += 1; v
        };
        out.push(DiffLineData {
            line_type: ty.into(),
            content: line.chars().skip(1).collect::<String>().into(),
            old_lineno: o,
            new_lineno: n,
        });
    }
    out
}
