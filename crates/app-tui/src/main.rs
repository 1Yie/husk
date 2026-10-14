//! `husk-tui` — ratatui terminal frontend for the agent kernel.
//!
//! Same kernel + UiEvent/UiCommand contract as the desktop shell and
//! `husk-cli`; main-screen inline viewport with committed cells pushed
//! into native scrollback (`insert_before`).

mod app;
mod cells;
mod composer;
mod events;
mod markdown;
mod modals;
mod render;
mod slash;
mod status;
#[cfg(test)]
mod tests;
mod transcript;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::Arc;

use agent_ipc::UiCommand;
use agent_kernel::channels::UiReceiver;
use agent_kernel::sandbox_prefs;
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_kernel::session_store::{SessionMeta, SessionStore};
use agent_llm::{AppConfig, ProviderFactory};

use app::App;

use crossterm::event::{Event, EventStream};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};

const USAGE: &str = "\
husk-tui — interactive terminal UI for the agent kernel

USAGE:
    husk-tui [OPTIONS]

OPTIONS:
    --workspace DIR       workspace root (default: cwd)
    -s, --session ID      resume session #ID directly (printed on exit)
    --prompt TEXT         send this prompt immediately at boot
    --provider NAME       provider key from config.toml (default: active)
    --model NAME          model override (default: active)
    --thinking LEVEL      reasoning effort (default: model default)
    --mode MODE           build | plan | goal  (default: build)
    --permission MODE     default | acceptEdits | auto | dontAsk |
                          bypassPermissions  (default: auto)
    --yes, -y             auto-approve approvals / first-answer questions
    --sandbox on|off      sandbox master switch (default: on)
    --sandbox-network M   auto | allow | deny
    --allow-domain HOST   extra egress allowlist entry (repeatable)
    --deny-domain HOST    egress denylist entry (repeatable)
    --version, --help

KEYS:
    Enter send · Ctrl+J/Alt+Enter newline · ↑/↓ history · Esc cancel
    Ctrl+C cancel running turn / quit when idle · Ctrl+D quit
    approvals: y / n / a(session-allow) · questions: digit or free text
";

struct Cli {
    workspace: PathBuf,
    session: Option<i64>,
    thinking: Option<String>,
    prompt: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    mode: String,
    permission: String,
    yes: bool,
    sandbox_enabled: Option<bool>,
    sandbox_network: Option<String>,
    allow_domains: Vec<String>,
    deny_domains: Vec<String>,
}

/// The host's own directories — the product names itself here.
fn app_paths() -> agent_paths::AppPaths {
    agent_paths::AppPaths::from_app_name("husk", ".husk")
        .unwrap_or_else(|| agent_paths::AppPaths::new("", "", ".husk"))
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        workspace: std::env::current_dir().map_err(|e| e.to_string())?,
        session: None,
        thinking: None,
        prompt: None,
        provider: None,
        model: None,
        mode: "build".into(),
        permission: "auto".into(),
        yes: false,
        sandbox_enabled: None,
        sandbox_network: None,
        allow_domains: Vec::new(),
        deny_domains: Vec::new(),
    };
    let mut args = std::env::args().skip(1).peekable();
    fn take(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
        it.next().ok_or_else(|| format!("{flag} needs a value"))
    }
    while let Some(a) = args.next() {
        match a.as_str() {
            "--workspace" => cli.workspace = PathBuf::from(take(&mut args, "--workspace")?),
            "--thinking" => cli.thinking = Some(take(&mut args, "--thinking")?),
            "--session" | "-s" => {
                cli.session = Some(
                    take(&mut args, "--session")?
                        .parse()
                        .map_err(|_| "--session needs a numeric id".to_string())?,
                )
            }
            "--prompt" | "-p" => cli.prompt = Some(take(&mut args, "--prompt")?),
            "--provider" => cli.provider = Some(take(&mut args, "--provider")?),
            "--model" => cli.model = Some(take(&mut args, "--model")?),
            "--mode" => cli.mode = take(&mut args, "--mode")?,
            "--permission" => cli.permission = take(&mut args, "--permission")?,
            "--yes" | "-y" => cli.yes = true,
            "--sandbox" => {
                let v = take(&mut args, "--sandbox")?;
                cli.sandbox_enabled = Some(match v.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--sandbox on|off, got {other}")),
                });
            }
            "--no-sandbox" => cli.sandbox_enabled = Some(false),
            "--sandbox-network" => {
                cli.sandbox_network = Some(take(&mut args, "--sandbox-network")?)
            }
            "--allow-domain" => cli.allow_domains.push(take(&mut args, "--allow-domain")?),
            "--deny-domain" => cli.deny_domains.push(take(&mut args, "--deny-domain")?),
            "--version" => {
                println!("husk-tui {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg {other:?}")),
        }
    }
    Ok(cli)
}

fn apply_sandbox_flags(cli: &Cli) -> Result<(), String> {
    let has = cli.sandbox_enabled.is_some()
        || cli.sandbox_network.is_some()
        || !cli.allow_domains.is_empty()
        || !cli.deny_domains.is_empty();
    if !has {
        return Ok(());
    }
    let mut lim = sandbox_prefs::current();
    if let Some(on) = cli.sandbox_enabled {
        lim.enabled = Some(on);
    }
    if let Some(net) = &cli.sandbox_network {
        lim.network = Some(
            sandbox_prefs::network_from_label(net)
                .ok_or_else(|| format!("--sandbox-network auto|allow|deny, got {net:?}"))?,
        );
    }
    lim.allowed_domains
        .extend(cli.allow_domains.iter().cloned());
    lim.denied_domains.extend(cli.deny_domains.iter().cloned());
    sandbox_prefs::set(lim);
    Ok(())
}

/// --provider/--model override on top of config.toml's active pair —
/// same logic as app-cli (`provider default_model` preferred).
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
    cfg.active_model.clone().unwrap_or_else(|| "default".into())
}

fn resolve_provider(
    cfg: &AppConfig,
    want_provider: Option<&str>,
    want_model: Option<&str>,
) -> Result<(Arc<dyn agent_llm::LlmProvider>, String, String), String> {
    if let Some(name) = want_provider {
        let pcfg = cfg
            .providers
            .get(name)
            .ok_or_else(|| format!("provider {name:?} not in config.toml"))?;
        let p = ProviderFactory::build(pcfg).map_err(|e| format!("provider {name:?}: {e}"))?;
        return Ok((p, name.into(), pick_model(cfg, pcfg, want_model)));
    }
    if let Some(name) = &cfg.active_provider {
        if let Some(pcfg) = cfg.providers.get(name) {
            if let Ok(p) = ProviderFactory::build(pcfg) {
                return Ok((p, name.clone(), pick_model(cfg, pcfg, want_model)));
            }
        }
    }
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

fn restore_terminal() {
    let _ = execute!(io::stdout(), DisableBracketedPaste, crossterm::cursor::Show);
    let _ = disable_raw_mode();
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        // Install cleanup before the next fallible setup operation.
        let guard = Self;
        execute!(io::stdout(), EnableBracketedPaste)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// One terminal per inline-height change — ratatui's Inline height is
/// fixed at construction, so transitions rebuild (cleared region + fresh
/// buffers = one clean full repaint).
fn new_terminal(height: u16) -> io::Result<ratatui::DefaultTerminal> {
    Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(height.max(2)),
        },
    )
}

/// Boot defaults and the provider catalog used to create/resume sessions.
struct SessionSeed {
    workspace: PathBuf,
    provider: Arc<dyn agent_llm::LlmProvider>,
    provider_name: String,
    model: String,
    permission: String,
    agent_mode: String,
    thinking: Option<String>,
    config: AppConfig,
}

/// `SessionConfig` for a fresh spawn (`meta = None`) or a resumed session
/// (`Some` — the persisted per-session choices win, mirroring the kernel's
/// own `spawn_actor` restore logic at session_manager.rs:1116).
fn make_cfg(seed: &SessionSeed, meta: Option<&SessionMeta>) -> Result<SessionConfig, String> {
    let model = meta
        .and_then(|m| m.model.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| seed.model.clone());
    let provider_name = meta
        .and_then(|m| m.provider.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| seed.provider_name.clone());
    let pcfg = seed.config.providers.get(&provider_name);
    let provider = if provider_name == seed.provider_name {
        seed.provider.clone()
    } else {
        let pcfg = pcfg.ok_or_else(|| format!("provider {provider_name:?} not in config.toml"))?;
        ProviderFactory::build(pcfg).map_err(|e| format!("provider {provider_name:?}: {e}"))?
    };
    let resolved = pcfg.map(|p| p.model_opts(&model));
    let model_params = resolved
        .as_ref()
        .map(agent_llm::ProviderFactory::model_params_from)
        .unwrap_or_default();
    Ok(SessionConfig {
        workspace_root: seed.workspace.clone(),
        paths: app_paths(),
        provider,
        provider_name,
        model,
        temperature: 1.0,
        permission_mode: meta
            .and_then(|m| m.permission_mode.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| seed.permission.clone()),
        agent_mode: meta
            .and_then(|m| m.agent_mode.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| seed.agent_mode.clone()),
        track_dirty: false,
        thinking_level: meta
            .map(|m| m.thinking_level.clone())
            .unwrap_or_else(|| seed.thinking.clone()),
        thinking_level_map: resolved.as_ref().and_then(|d| d.thinking_level_map.clone()),
        context_window: resolved.as_ref().and_then(|d| d.context_window),
        model_input: resolved.map(|d| d.input.clone()).unwrap_or_default(),
        compact_at: None,
        model_params: Some(model_params),
        plugins: None,
        queued_prompts: meta.map(|m| m.queued_prompts.clone()).unwrap_or_default(),
        memory_enabled: true,
        memory_distill: true,
    })
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
    apply_sandbox_flags(&cli).map_err(anyhow::Error::msg)?;

    if !io::stdout().is_terminal() {
        eprintln!("husk-tui needs a terminal; use husk-cli --headless for pipes");
        std::process::exit(2);
    }

    let paths = app_paths();
    agent_kernel::session_store::init_data_dir(paths.data_dir.clone());
    let app_cfg = agent_llm::AppConfig::load(&paths.config_dir, None).unwrap_or_default();
    let (provider, provider_name, model) =
        match resolve_provider(&app_cfg, cli.provider.as_deref(), cli.model.as_deref()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
        };
    // `/model`'s picker needs provider → model-id lists — the same
    // config tables `resolve_provider` reads.
    let model_catalog: Vec<(String, Vec<String>)> = app_cfg
        .providers
        .iter()
        .map(|(n, p)| {
            (
                n.clone(),
                p.models.iter().map(|m| m.id().to_string()).collect(),
            )
        })
        .collect();

    let seed = SessionSeed {
        workspace: cli.workspace.clone(),
        provider,
        provider_name: provider_name.clone(),
        model: model.clone(),
        permission: cli.permission.clone(),
        agent_mode: cli.mode.clone(),
        thinking: cli.thinking.clone(),
        config: app_cfg,
    };
    // Allocate a real id BEFORE spawning — `SessionActor::spawn` passes 0
    // and every run would clobber the same store row. `-s <id>` resumes a
    // persisted session exactly like the `/resume` picker does mid-run.
    let store = SessionStore::open(&cli.workspace).ok();
    let mut boot_meta: Option<SessionMeta> = None;
    let mut boot_history: Option<Vec<agent_llm::ChatMessage>> = None;
    let session_id = match cli.session {
        Some(id) if store.as_ref().and_then(|s| s.load_history(id)).is_some() => {
            let s = store.as_ref().unwrap();
            let meta = s.list().into_iter().find(|m| m.id == id);
            let history = s.load_history(id).unwrap();
            let _ = s.set_last_active(id);
            boot_history = Some(history.clone());
            eprintln!("resumed session #{id}");
            boot_meta = meta;
            id
        }
        Some(id) => {
            eprintln!("session #{id} not found — starting fresh");
            let nid = store.as_ref().map(|s| s.next_id()).unwrap_or(0);
            nid
        }
        None => {
            let nid = store.as_ref().map(|s| s.next_id()).unwrap_or(0);
            nid
        }
    };
    let cfg = make_cfg(&seed, boot_meta.as_ref()).map_err(anyhow::Error::msg)?;
    let provider_name = cfg.provider_name.clone();
    let model = cfg.model.clone();
    let permission = cfg.permission_mode.clone();
    let agent_mode = cfg.agent_mode.clone();
    let thinking = cfg.thinking_level.clone();
    let (mut actor, channels) = match &boot_history {
        Some(history) => SessionActor::resume(cfg, session_id, history.clone()),
        None => SessionActor::spawn_with_id(cfg, session_id),
    };
    let cmd_tx = actor.command_sender();
    let ev_rx: UiReceiver = channels.event_rx;
    // Fast-path handles must be captured BEFORE actor.run() moves it —
    // mid-turn the cmd pump is blocked, so approvals/cancels queue forever
    // if they travel by channel (same reason the IPC bridge writes these
    // slots directly).
    let decision_slot = actor.decision_writer();
    let ask_channel = actor.ask_channel();
    let cancel_flag = actor.cancel_writer();
    let queue = actor.queue_writer();

    let sandbox_id = if !sandbox_prefs::current().enabled() {
        "off".to_string()
    } else {
        let (b, _) = agent_sandbox::detect_backend();
        b.id().to_string()
    };

    // terminal setup — raw mode + bracketed paste, no alternate screen
    // (scrollback stays terminal-native)
    let terminal_guard = TerminalGuard::enter()?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        hook(info);
    }));

    let mut app = App::new(
        cmd_tx.clone(),
        ev_rx,
        decision_slot,
        ask_channel,
        cancel_flag,
        queue,
        provider_name,
        model,
        sandbox_id,
        cli.yes,
        cli.workspace.clone(),
        permission,
        agent_mode,
        thinking,
        model_catalog,
    );
    app.session_id = session_id;
    // The actor owns the turn engine — it runs as a task for the app's
    // lifetime; dropping cmd/event channels on exit ends it.
    tokio::spawn(async move { actor.run().await });

    // Startup card — committed straight to scrollback.
    app.commit(crate::cells::Cell::Welcome {
        model: format!("{}/{}", app.provider_name, app.model),
        sandbox: app.sandbox_id.clone(),
        workspace: cli.workspace.display().to_string(),
    });
    // `-s` boot — replay the resumed history as real cells, not a notice.
    if let Some(h) = &boot_history {
        commit_history(&mut app, h, session_id);
    }

    let code = run_loop(&mut app, cli.prompt, &seed).await;

    drop(terminal_guard);
    if app.session_id > 0 {
        // The inline viewport paints up to the last terminal row — the hint
        // would overwrite its bottom border. A blank line scrolls it up.
        println!();
        println!("回到此会话: husk-tui -s {}", app.session_id);
    }
    std::process::exit(code);
}

/// `5:33 AM` from a persisted epoch-ms timestamp (same libc localtime as
/// `hm_now`, just not "now").
fn ts_hm(ts_ms: Option<i64>) -> String {
    let Some(ms) = ts_ms else {
        return String::new();
    };
    unsafe {
        let secs = (ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        let (h24, m) = (tm.tm_hour, tm.tm_min);
        let (h12, ap) = match h24 {
            0 => (12, "AM"),
            h if h < 12 => (h, "AM"),
            12 => (12, "PM"),
            h => (h - 12, "PM"),
        };
        format!("{h12}:{m:02} {ap}")
    }
}

/// Replay a persisted history into transcript cells — user/assistant turns
/// keep their original timestamps; tool calls re-render as ✓/✗ rows paired
/// with their `Role::Tool` results by `tool_call_id`. System/Tool messages
/// are wire state, not transcript content. A resumed `todo` result also
/// restores the persistent panel.
fn commit_history(app: &mut App, history: &[agent_llm::ChatMessage], session_id: i64) {
    use agent_llm::Role;
    let mut results: std::collections::HashMap<&str, (&str, bool)> =
        std::collections::HashMap::new();
    for m in history {
        if m.role == Role::Tool {
            if let Some(id) = m.tool_call_id.as_deref() {
                results.insert(
                    id,
                    (
                        m.content.as_deref().unwrap_or(""),
                        m.is_error.unwrap_or(false),
                    ),
                );
            }
        }
    }
    app.commit(crate::cells::Cell::Note(format!(
        "── 已恢复会话 #{session_id} ──"
    )));
    for m in history {
        match m.role {
            Role::User => {
                if let Some(text) = &m.content {
                    if !text.trim().is_empty() {
                        app.commit(crate::cells::Cell::User {
                            text: text.clone(),
                            time: ts_hm(m.ts),
                        });
                    }
                }
            }
            Role::Assistant => {
                if let Some(calls) = &m.tool_calls {
                    for c in calls {
                        let (content, is_err) =
                            results.get(c.id.as_str()).copied().unwrap_or(("", false));
                        // Prefer the actionable arg (command/path/prompt…)
                        // over raw JSON — matches live `✓ bash  echo 1`.
                        let preview = serde_json::from_str::<serde_json::Value>(&c.arguments)
                            .ok()
                            .and_then(|v| {
                                v.as_object().and_then(|o| {
                                    [
                                        "command", "cmd", "path", "file", "pattern", "prompt",
                                        "query", "url",
                                    ]
                                    .iter()
                                    .find_map(|k| o.get(*k).and_then(|x| x.as_str()))
                                    .map(str::to_string)
                                })
                            })
                            .unwrap_or_else(|| {
                                c.arguments.split_whitespace().collect::<Vec<_>>().join(" ")
                            });
                        app.commit(crate::cells::Cell::Tool {
                            name: c.name.clone(),
                            preview: preview.chars().take(60).collect(),
                            ok: !is_err,
                            tail: if is_err || c.name == "todo" {
                                app::fail_tail(content)
                            } else {
                                Vec::new()
                            },
                            secs: None,
                        });
                        if c.name == "todo" && !is_err {
                            app.todos = events::parse_todos(content);
                        }
                    }
                }
                if let Some(text) = &m.content {
                    if !text.trim().is_empty() {
                        app.commit(crate::cells::Cell::Agent {
                            md: text.clone(),
                            time: ts_hm(m.ts),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Mirror the exact configuration passed to the new actor.
fn apply_cfg_labels(app: &mut App, cfg: &SessionConfig) {
    app.provider_name = cfg.provider_name.clone();
    app.model = cfg.model.clone();
    app.permission_mode = cfg.permission_mode.clone();
    app.agent_mode = cfg.agent_mode.clone();
    app.thinking_level = cfg.thinking_level.clone();
}

/// `/new` — allocate a fresh row id, spawn a clean actor, swap plumbing.
/// Same shape as `resume_into` minus history; the old actor dies with its
/// channels on drop.
async fn new_into(app: &mut App, seed: &SessionSeed) {
    let cfg = match make_cfg(seed, None) {
        Ok(cfg) => cfg,
        Err(e) => {
            app.commit(crate::cells::Cell::Err(e));
            return;
        }
    };
    let Some(store) = SessionStore::open_existing(&seed.workspace) else {
        app.commit(crate::cells::Cell::Err("工作区没有会话存储".into()));
        return;
    };
    let id = store.next_id();
    let _ = store.set_last_active(id);
    apply_cfg_labels(app, &cfg);
    let (mut actor, channels) = SessionActor::spawn_with_id(cfg, id);
    let cmd_tx = actor.command_sender();
    let ev_rx = channels.event_rx;
    let decision_slot = actor.decision_writer();
    let ask_channel = actor.ask_channel();
    let cancel_flag = actor.cancel_writer();
    let queue = actor.queue_writer();
    tokio::spawn(async move { actor.run().await });
    app.swap_session(
        cmd_tx,
        ev_rx,
        decision_slot,
        ask_channel,
        cancel_flag,
        queue,
    );
    app.session_id = id;
    app.commit(crate::cells::Cell::Note(format!("已开启新会话 #{id}")));
}

/// `/resume` pick resolved — rebuild the actor on this workspace's store
/// and swap the app's live plumbing. Mirrors the boot spawn minus argv:
/// the new actor gets its own pump+writer handles; the transcript keeps
/// every committed line (they're already on the terminal's scrollback).
async fn resume_into(app: &mut App, seed: &SessionSeed, id: i64) {
    let Some(store) = SessionStore::open_existing(&seed.workspace) else {
        app.commit(crate::cells::Cell::Err("工作区没有会话存储".into()));
        return;
    };
    let meta = store.list().into_iter().find(|m| m.id == id);
    let Some(history) = store.load_history(id) else {
        app.commit(crate::cells::Cell::Err(format!("会话 #{id} 无历史可恢复")));
        return;
    };
    let cfg = match make_cfg(seed, meta.as_ref()) {
        Ok(cfg) => cfg,
        Err(e) => {
            app.commit(crate::cells::Cell::Err(e));
            return;
        }
    };
    let _ = store.set_last_active(id);

    commit_history(app, &history, id);
    apply_cfg_labels(app, &cfg);
    let (mut actor, channels) = SessionActor::resume(cfg, id, history);
    let cmd_tx = actor.command_sender();
    let ev_rx = channels.event_rx;
    let decision_slot = actor.decision_writer();
    let ask_channel = actor.ask_channel();
    let cancel_flag = actor.cancel_writer();
    let queue = actor.queue_writer();
    tokio::spawn(async move { actor.run().await });

    app.swap_session(
        cmd_tx,
        ev_rx,
        decision_slot,
        ask_channel,
        cancel_flag,
        queue,
    );
    app.session_id = id;
}

async fn run_loop(app: &mut App, boot_prompt: Option<String>, seed: &SessionSeed) -> i32 {
    // Terminal init BEFORE the event stream: `Viewport::Inline` anchors to
    // the backend cursor row via a CPR query (ESC[6n) — crossterm's
    // EventStream owns a stdin reader thread that would steal the reply.
    // A FIXED inline height avoids ever re-querying (rebuilds would race
    // the reader); the activity region collapses to blank when idle — the
    // honest v1 trade (a pausable event reader + height rebuilds is the
    // follow-up).
    let (_, th) = crossterm::terminal::size().unwrap_or((80, 24));
    let viewport_h: u16 = (th / 2).clamp(6, 16);
    let mut term: ratatui::DefaultTerminal = match new_terminal(viewport_h) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("terminal init failed: {e}");
            return 1;
        }
    };
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frames = tokio::time::interval(std::time::Duration::from_millis(34));
    frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut draw_due = true;
    let mut dirty = true;

    if let Some(p) = boot_prompt {
        let _ = app.cmd_tx.send(UiCommand::Prompt { text: p }).await;
    }

    loop {
        // Consume events between frames; rendering is capped at ~30 fps.
        if draw_due {
            draw_due = false;
            dirty = false;
            // commit pending transcript lines into scrollback first
            if let Err(e) = app.flush_scrollback(&mut term) {
                let _ = e;
            }
            let (tw, _th) = crossterm::terminal::size().unwrap_or((80, 24));
            app_set_width(app, tw);
            if let Err(e) = term.draw(|f| render::draw(f, app)) {
                eprintln!("\r\n[tui] draw failed: {e}");
                break;
            }
            // The hardware cursor belongs in the composer; while a modal owns
            // the keys there is nowhere sane to park it — hide instead of
            // leaving a stray block wherever the last write ended.
            if app.modal.is_some() {
                let _ = term.hide_cursor();
            } else {
                let _ = term.show_cursor();
            }
        }
        if app.quit {
            break;
        }
        // `/resume` — the picker modal parked the request; rebuild now.
        if let Some(id) = app.take_resume_request() {
            resume_into(app, seed, id).await;
            dirty = true;
        }
        if app.take_new_request() {
            new_into(app, seed).await;
            dirty = true;
        }

        tokio::select! {
            // Input and frame deadlines must not be starved by queued deltas.
            biased;
            ev = events.next() => {
                match ev {
                    Some(Ok(Event::Resize(w, h))) => {
                        // ratatui's inline resize queries the cursor (CPR). The
                        // EventStream reader would steal that reply, so drop it
                        // for the duration of the resize and re-arm afterwards.
                        drop(events);
                        let _ = term.resize(ratatui::layout::Rect::new(0, 0, w, h));
                        events = EventStream::new();
                        app.set_width(w);
                        dirty = true;
                        draw_due = true;
                    }
                    Some(Ok(e)) => {
                        for c in app.on_term_event(e) {
                            app.dispatch(c).await;
                        }
                        dirty = true;
                    }
                    Some(Err(_)) | None => {
                        eprintln!("\r\n[tui] input stream ended");
                        break;
                    }
                }
            }
            _ = frames.tick() => {
                draw_due = dirty;
            }
            _ = tick.tick() => {
                app.spinner = app.spinner.wrapping_add(1);
                app.on_tick();
                dirty = true;
            }
            ev = app.ev_rx.recv() => {
                match ev {
                    Some(e) => {
                        app.on_ui_event(e);
                        dirty = true;
                    }
                    None => {
                        eprintln!("\r\n[tui] event channel closed");
                        break;
                    }
                }
            }
        }
    }
    if app.exit_code != 0 {
        app.exit_code
    } else if app.state.is_active() {
        130
    } else {
        0
    }
}

fn app_set_width(app: &mut App, w: u16) {
    // width setter is private-ish; channel through a pub fn on App
    app.set_width(w);
}
