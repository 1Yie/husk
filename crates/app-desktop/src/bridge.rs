//! `bridge` — kernel wiring for the iced shell.
//!
//! iced's Elm loop can't `await` tokio channels, so the kernel's `UiEvent`
//! stream is forwarded to a `std::sync::mpsc` that an iced `Subscription`
//! polls each frame. Kernel spawn/provider resolution is unchanged from the
//! Slint version — only the UI-side pump differs.

use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use agent_ipc::UiEvent;
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_llm::{AppConfig, ProviderFactory};

use crate::ui::state::StatsRow;

/// Everything the UI needs to attach to a live kernel.
pub struct KernelHandles {
    pub cmd_tx: tokio::sync::mpsc::Sender<agent_ipc::UiCommand>,
    pub decision: Arc<Mutex<Option<bool>>>,
    pub steer_tx: tokio::sync::mpsc::Sender<String>,
    /// Kernel events, forwarded to a std-mpsc the UI subscription drains.
    pub event_rx: std_mpsc::Receiver<UiEvent>,
    pub stats: StatsRow,
    pub sandbox_unsafe: bool,
}

/// Resolve the active provider from `config.toml`.
fn resolve_provider(
    cfg: &AppConfig,
) -> (Arc<dyn agent_llm::LlmProvider>, String, String) {
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
    (
        Arc::new(agent_llm::adapters::MockProvider::new()),
        "mock".into(),
        "mock (no provider configured)".into(),
    )
}

/// Spawn the kernel + the tokio→std-mpsc event forwarder.
/// Returns the handles `App::boot` consumes. Runs the actor on a dedicated
/// `kernel-rt` thread (no tokio reactor on the iced main thread).
pub fn spawn_kernel() -> KernelHandles {
    let cfg = AppConfig::load(None).unwrap_or_default();
    let (provider, model, provider_label) = resolve_provider(&cfg);

    let (mut actor, channels) = SessionActor::spawn(SessionConfig {
        workspace_root: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        provider,
        model: model.clone(),
        temperature: 1.0,
        permission_mode: "default".into(),
        track_dirty: true,
    });

    let cmd_tx = actor.command_sender();
    let decision = actor.decision_writer();
    let steer_tx = actor.steer_writer();

    // Actor run() on a dedicated tokio runtime thread.
    std::thread::Builder::new()
        .name("kernel-rt".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move { actor.run().await });
        })
        .expect("spawn kernel-rt");

    // tokio event_rx → std-mpsc forwarder, on its own runtime thread.
    let (ui_tx, ui_rx) = std_mpsc::channel::<UiEvent>();
    {
        let mut rx = channels.event_rx;
        std::thread::Builder::new()
            .name("ui-forward".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio rt");
                rt.block_on(async move {
                    while let Some(ev) = rx.recv().await {
                        if ui_tx.send(ev).is_err() {
                            break;
                        }
                    }
                });
            })
            .expect("spawn ui-forward");
    }

    let (_b, loud) = agent_sandbox::detect_backend();

    KernelHandles {
        cmd_tx,
        decision,
        steer_tx,
        event_rx: ui_rx,
        stats: StatsRow {
            agent_state: "Idle".into(),
            permission_mode: "default".into(),
            active_provider: provider_label,
            active_model: model,
            ..Default::default()
        },
        sandbox_unsafe: loud,
    }
}
