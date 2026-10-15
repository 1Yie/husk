//! `app-cli` — headless CLI frontend for the agent kernel.
//!
//! Same kernel as the desktop shell, stdout event pump instead of iced.
//! Unlike the desktop, a `--prompt` run is one-shot: it exits on
//! `Finished`/`Failed` with a real exit code, so the binary composes with
//! pipes and CI.
//!
//! ```
//! husk-cli --headless --prompt "explain main.rs"          # stream + exit
//! echo "hi" | husk-cli --headless                         # stdin prompt
//! husk-cli --headless --prompt t --json --timeout 120     # JSONL events
//! husk-cli --headless -p t --allow-domain crates.io --yes # sandboxed+auto
//! ```

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use agent_ipc::UiEvent;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};
use husk_host::sandbox_prefs;
use agent_llm::{AppConfig, ProviderFactory};

const USAGE: &str = "\
husk-cli — headless frontend for the agent kernel

USAGE:
    husk-cli --headless [OPTIONS]
    echo \"prompt\" | husk-cli --headless [OPTIONS]

OPTIONS:
    --workspace DIR         workspace root (default: cwd)
    --prompt TEXT           run this prompt, then exit
    --prompt-file PATH      read prompt from a file ('-' = stdin)
    --provider NAME         provider key from config.toml (default: active)
    --model NAME            model override (default: active)
    --mode MODE             build | plan | goal  (default: build)
    --permission MODE       default | acceptEdits | auto | dontAsk |
                            bypassPermissions  (default: auto)
    --timeout SECS          wall-clock cap for the turn (0 = none)
    --yes, -y               auto-approve pending approvals and answer
                            questions with the first option. Without it,
                            approvals are auto-DENIED (safe default).
    --json                  emit each UiEvent as one JSON line on stdout
    --verbose               print state transitions and tool results
    --sandbox on|off        master switch — off runs every process tool
                            through the loud unsandboxed `none` backend
    --sandbox-network M     auto | allow | deny  (default: auto)
    --allow-domain HOST     allow extra egress domains for sandboxed
                            commands ('*.example.com', 'host:443',
                            repeatable)
    --deny-domain HOST      deny egress domains (repeatable)
    --version, --help

EXIT CODES:
    0   turn finished
    1   turn failed / provider unavailable
    2   usage error
    124 timeout hit
    130 interrupted (Ctrl+C)
";

#[derive(Default)]
struct Cli {
    workspace: PathBuf,
    prompt: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    mode: String,
    permission: String,
    timeout: u64,
    yes: bool,
    json: bool,
    verbose: bool,
    sandbox_network: Option<String>,
    sandbox_enabled: Option<bool>,
    allow_domains: Vec<String>,
    deny_domains: Vec<String>,
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        mode: "build".into(),
        permission: "auto".into(),
        ..Default::default()
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--version") {
        println!("husk-cli {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    if !args.iter().any(|a| a == "--headless") {
        return Err("missing --headless (the GUI binary is `agent-desktop`)".into());
    }
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        // `--flag=value` and `--flag value` both accepted.
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) => (f, Some(v.to_string())),
            None => (a, None),
        };
        let mut take = || -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--headless" => {}
            "--workspace" => cli.workspace = PathBuf::from(take()?),
            "--prompt" | "-p" => cli.prompt = Some(take()?),
            "--prompt-file" => {
                let path = take()?;
                let text = if path == "-" {
                    read_stdin()
                } else {
                    std::fs::read_to_string(&path)
                        .map_err(|e| format!("prompt-file {path}: {e}"))?
                };
                cli.prompt = Some(text);
            }
            "--provider" => cli.provider = Some(take()?),
            "--model" => cli.model = Some(take()?),
            "--mode" => cli.mode = take()?,
            "--permission" => cli.permission = take()?,
            "--timeout" => {
                cli.timeout = take()?
                    .parse()
                    .map_err(|_| "--timeout expects seconds".to_string())?
            }
            "--yes" | "-y" => cli.yes = true,
            "--json" => cli.json = true,
            "--verbose" | "-v" => cli.verbose = true,
            "--sandbox" => {
                let v = take()?;
                cli.sandbox_enabled = Some(match v.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--sandbox on|off, got {other}")),
                });
            }
            "--no-sandbox" => cli.sandbox_enabled = Some(false),
            "--sandbox-network" => cli.sandbox_network = Some(take()?),
            "--allow-domain" => cli.allow_domains.push(take()?),
            "--deny-domain" => cli.deny_domains.push(take()?),
            other => return Err(format!("unknown flag {other}")),
        }
        i += 1;
    }
    Ok(cli)
}

fn read_stdin() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    s.trim_end().to_string()
}

/// CLI flags → process-global sandbox prefs — the kernel folds them into
/// every tool run via `sandbox_prefs::apply_to`.
fn apply_sandbox_flags(cli: &Cli) -> Result<(), String> {
    let has_flags = cli.sandbox_enabled.is_some()
        || cli.sandbox_network.is_some()
        || !cli.allow_domains.is_empty()
        || !cli.deny_domains.is_empty();
    if !has_flags {
        return Ok(());
    }
    let mut lim = sandbox_prefs::current();
    if let Some(on) = cli.sandbox_enabled {
        lim.enabled = Some(on);
    }
    if let Some(net) = &cli.sandbox_network {
        match net.as_str() {
            "auto" | "allow" | "deny" => {
                lim.network = sandbox_prefs::network_from_label(net)
            }
            other => return Err(format!("--sandbox-network: bad value {other}")),
        }
    }
    for d in &cli.allow_domains {
        if !lim.allowed_domains.contains(d) {
            lim.allowed_domains.push(d.clone());
        }
    }
    for d in &cli.deny_domains {
        if !lim.denied_domains.contains(d) {
            lim.denied_domains.push(d.clone());
        }
    }
    sandbox_prefs::set(lim);
    Ok(())
}

/// --provider/--model override on top of config.toml's active pair.
/// Model choice: explicit --model > provider's default_model > config's
/// active_model (only when it actually belongs to this provider).
fn pick_model(
    cfg: &AppConfig,
    pcfg: &agent_llm::ProviderConfig,
    want_model: Option<&str>,
) -> String {
    if let Some(m) = want_model {
        return m.into();
    }
    if let Some(d) = &pcfg.default_model {
        return d.clone();
    }
    cfg.active_model
        .clone()
        .unwrap_or_else(|| "default".into())
}

fn resolve_provider(
    cfg: &AppConfig,
    want_provider: Option<&str>,
    want_model: Option<&str>,
) -> Result<(Arc<dyn agent_llm::LlmProvider>, String, String), String> {
    // An explicitly named provider must resolve — no silent fallback that
    // could bill or hit a different account.
    if let Some(name) = want_provider {
        let pcfg = cfg
            .providers
            .get(name)
            .ok_or_else(|| format!("provider {name:?} not in config.toml"))?;
        let p = ProviderFactory::build(pcfg)
            .map_err(|e| format!("provider {name:?}: {e}"))?;
        return Ok((p, name.into(), pick_model(cfg, pcfg, want_model)));
    }
    // Active pair first…
    if let Some(name) = &cfg.active_provider {
        if let Some(pcfg) = cfg.providers.get(name) {
            if let Ok(p) = ProviderFactory::build(pcfg) {
                return Ok((
                    p,
                    name.clone(),
                    pick_model(cfg, pcfg, want_model),
                ));
            }
        }
    }
    // …then any configured provider.
    for (n, pcfg) in &cfg.providers {
        if let Ok(p) = ProviderFactory::build(pcfg) {
            return Ok((p, n.clone(), pick_model(cfg, pcfg, want_model)));
        }
    }
    Ok((
        Arc::new(agent_llm::provider::UnconfiguredProvider),
        "cli".into(),
        want_model.unwrap_or("default").into(),
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    apply_sandbox_flags(&cli).map_err(|e| anyhow::anyhow!(e))?;

    // Prompt may also arrive on a piped stdin.
    let prompt = match cli.prompt.clone() {
        Some(p) if !p.is_empty() => Some(p),
        _ if !std::io::stdin().is_terminal() => {
            let s = read_stdin();
            (!s.is_empty()).then_some(s)
        }
        _ => None,
    };

    let paths = agent_paths::AppPaths::from_app_name("husk", ".husk")
        .unwrap_or_else(|| agent_paths::AppPaths::new("", "", ".husk"));
    husk_host::office::init_store(paths.data_dir.clone());
    let app_cfg = agent_llm::AppConfig::load(&paths.config_dir, None).unwrap_or_default();
    let pname = cli.provider.as_deref().or(app_cfg.active_provider.as_deref());
    let (provider, provider_name, model) =
        match resolve_provider(&app_cfg, cli.provider.as_deref(), cli.model.as_deref()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        };
    let pcfg = pname.and_then(|p| app_cfg.providers.get(p));
    let resolved = pcfg.and_then(|p| p.find_model(&model)).map(|_| {
        pcfg.map(|p| p.model_opts(&model)).unwrap_or_default()
    });
    let model_params = resolved
        .as_ref()
        .map(agent_llm::ProviderFactory::model_params_from)
        .unwrap_or_default();
    let thinking_level_map = resolved
        .as_ref()
        .and_then(|d| d.thinking_level_map.clone());
    let context_window = resolved.as_ref().and_then(|d| d.context_window);
    let model_input = resolved.map(|d| d.input.clone()).unwrap_or_default();

    let cfg = SessionConfig {
        workspace_root: cli.workspace.clone(),
        paths,
        provider,
        provider_name,
        model,
        temperature: 1.0,
        permission_mode: cli.permission.clone(),
        agent_mode: cli.mode.clone(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map,
        context_window,
        model_input,
        compact_at: None,
        model_params: Some(model_params),
        plugins: None,
        queued_prompts: Vec::new(),
        memory_enabled: true,
        memory_distill: true,
    };
    let session_id = agent_store::SessionStore::open(&cli.workspace)
        .map_err(|error| anyhow::anyhow!("session storage: {error}"))?
        .next_id();
    let (mut actor, channels) = SessionActor::spawn_with_id(cfg, session_id);
    let cmd_tx = actor.command_sender();
    let mut ev_rx = channels.event_rx;

    // Event pump. Owns the exit decision: a prompt run terminates on
    // Finished/Failed; an idle session (no prompt) stays alive until
    // Ctrl+C like before.
    let decision_slot = actor.decision_writer();
    let ask_channel = actor.ask_channel();
    let (yes, json, verbose, one_shot) = (cli.yes, cli.json, cli.verbose, prompt.is_some());
    let pump = tokio::spawn(async move {
        let mut code = 0i32;
        while let Some(ev) = ev_rx.recv().await {
            if json {
                println!("{}", serde_json::to_string(&ev).unwrap_or_default());
            } else {
                match &ev {
                    UiEvent::TextDelta { text, .. } => print!("{text}"),
                    UiEvent::ReasoningDelta { text, .. } => {
                        eprint!("\x1b[2m{text}\x1b[0m")
                    }
                    UiEvent::StateChanged(s) if verbose => {
                        eprintln!("\n[state] {s:?}")
                    }
                    UiEvent::SystemMessage(m) => eprintln!("\n[sys] {m}"),
                    UiEvent::Error(e) => eprintln!("[err] {e}"),
                    UiEvent::AssistantMessage(m) => println!("\n{m}"),
                    UiEvent::ToolCallStarted { name, args_preview, .. } => {
                        eprintln!("[tool →] {name} {args_preview}")
                    }
                    UiEvent::ToolCallFinished { name, ok, content, .. } => {
                        if verbose {
                            eprintln!("[tool ✓] {name} ok={ok}\n{content}")
                        } else {
                            eprintln!("[tool ✓] {name} ok={ok}")
                        }
                    }
                    UiEvent::ApprovalRequested { tool_name, .. } => {
                        eprintln!("[approval] {tool_name} — {}", if yes { "auto-approved" } else { "auto-denied" })
                    }
                    UiEvent::QuestionAsked { question, .. } => {
                        eprintln!("[question] {question}")
                    }
                    _ => {}
                }
            }
            auto_decide(&ev, yes, &decision_slot, &ask_channel);
            match &ev {
                UiEvent::StateChanged(agent_ipc::AgentState::Finished) if one_shot => break,
                UiEvent::StateChanged(agent_ipc::AgentState::Failed(e)) if one_shot => {
                    eprintln!("[failed] {e}");
                    code = 1;
                    break;
                }
                _ => {}
            }
            let _ = std::io::stdout().flush();
        }
        code
    });

    let run = tokio::spawn(async move { actor.run().await });
    if let Some(p) = prompt {
        let _ = cmd_tx.send(UiCommand::Prompt { text: p }).await;
    }

    let code = if cli.timeout > 0 && one_shot {
        match tokio::time::timeout(
            std::time::Duration::from_secs(cli.timeout),
            pump,
        )
        .await
        {
            Ok(Ok(c)) => c,
            Ok(Err(_)) => 1,
            Err(_) => {
                eprintln!("[timeout] {}s exceeded", cli.timeout);
                124
            }
        }
    } else {
        // No prompt → idle daemon; Ctrl+C exits.
        tokio::select! {
            r = pump => r.unwrap_or(1),
            _ = tokio::signal::ctrl_c() => 130,
        }
    };
    run.abort();
    std::process::exit(code);
}

/// The actor's command pump is occupied by the waiting turn, so headless
/// decisions must reach the engine's shared slots directly.
fn auto_decide(
    ev: &UiEvent,
    yes: bool,
    decision_slot: &std::sync::Mutex<Option<(u64, bool)>>,
    ask_channel: &agent_kernel::tools::registry::AskChannel,
) {
    match ev {
        UiEvent::ApprovalRequested { request_id, .. } => {
            *decision_slot.lock().unwrap() = Some((*request_id, yes));
        }
        UiEvent::QuestionAsked { request_id, options, .. } => {
            let answer = if yes {
                options.first().map(|o| o.label.clone()).unwrap_or_default()
            } else {
                String::new()
            };
            ask_channel.answer(*request_id, answer);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
