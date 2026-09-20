//! `app-cli` — headless CLI frontend for the agent kernel.
//!
//! Contract (workspace-layout §4): a real binary from day one — thin
//! adapter over `agent-ipc` events, sharing the same kernel as
//! `app-desktop`. `--headless` prints `UiEvent`s to stdout; future remote
//! worker mode pipes them over a socket.

use std::path::PathBuf;
use std::sync::Arc;

use agent_ipc::{UiCommand, UiEvent};
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_llm::{AppConfig, ProviderFactory};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let workspace = args
        .iter()
        .position(|a| a == "--workspace")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let prompt = args
        .iter()
        .position(|a| a == "--prompt")
        .and_then(|i| args.get(i + 1))
        .cloned();

    if !headless {
        eprintln!("usage: agent-cli --headless [--workspace DIR] [--prompt TEXT]");
        eprintln!("       (the GUI binary is `agent-desktop`)");
        std::process::exit(2);
    }

    // Headless = same kernel, stdout event pump instead of iced. Provider
    // resolves from `config.toml` — same path as the desktop `--live`.
    let app_cfg = AppConfig::load(None).unwrap_or_default();
    let (provider, model) = resolve_provider(&app_cfg);
    let mentry = app_cfg
        .active_provider
        .as_ref()
        .and_then(|p| app_cfg.providers.get(p))
        .and_then(|p| p.find_model(&model));
    let detailed = mentry.and_then(|m| m.detailed());
    let thinking_level_map = detailed
        .as_ref()
        .and_then(|d| d.thinking_level_map.clone());
    let context_window = detailed.as_ref().and_then(|d| d.context_window);
    let model_input = detailed.map(|d| d.input.clone()).unwrap_or_default();
    let cfg = SessionConfig {
        workspace_root: workspace,
        provider,
        model,
        temperature: 1.0,
        permission_mode: "auto".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map,
        context_window,
        model_input,
    };
    let (mut actor, channels) = SessionActor::spawn(cfg);
    let cmd_tx = actor.command_sender();
    let mut ev_rx = channels.event_rx;

    // Event pump — print each UiEvent as a line (JSON for the future
    // `--remote` worker mode to reuse verbatim).
    let pump = tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            match &ev {
                UiEvent::TextDelta(t) => print!("{t}"),
                UiEvent::ReasoningDelta(t) => eprint!("\x1b[2m{t}\x1b[0m"),
                UiEvent::StateChanged(s) => eprintln!("\n[state] {s:?}"),
                UiEvent::SystemMessage(m) => eprintln!("\n[sys] {m}"),
                UiEvent::Error(e) => eprintln!("[err] {e}"),
                UiEvent::AssistantMessage(m) => println!("\n{m}"),
                UiEvent::ToolCallStarted { name, .. } => eprintln!("[tool →] {name}"),
                UiEvent::ToolCallFinished { name, ok, .. } => {
                    eprintln!("[tool ✓] {name} ok={ok}")
                }
                _ => {}
            }
            // Flush on delta so streaming reads live.
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    });

    // Drive the actor; send the prompt if one was given.
    let run = tokio::spawn(async move { actor.run().await });
    if let Some(p) = prompt {
        let _ = cmd_tx.send(UiCommand::Prompt { text: p }).await;
    }
    // `--headless` without a prompt just idles until killed — the worker
    // mode is a future socket server; for now the pump reports readiness.
    let _ = tokio::signal::ctrl_c().await;
    run.abort();
    pump.abort();
    Ok(())
}

/// `config.toml` → provider + model — same resolution order as the desktop
/// (`active_provider` → first available → mock).
fn resolve_provider(cfg: &AppConfig) -> (Arc<dyn agent_llm::LlmProvider>, String) {
    if let Some(name) = &cfg.active_provider {
        if let Some(pcfg) = cfg.providers.get(name) {
            if let Ok(p) = ProviderFactory::build(pcfg) {
                let model = cfg.active_model.clone()
                    .or_else(|| pcfg.default_model.clone())
                    .unwrap_or_else(|| "default".into());
                return (p, model);
            }
        }
    }
    for (_name, pcfg) in &cfg.providers {
        if let Ok(p) = ProviderFactory::build(pcfg) {
            let model = cfg.active_model.clone()
                .or_else(|| pcfg.default_model.clone())
                .unwrap_or_else(|| "default".into());
            return (p, model);
        }
    }
    (Arc::new(agent_llm::adapters::MockProvider::new()), "mock".into())
}
