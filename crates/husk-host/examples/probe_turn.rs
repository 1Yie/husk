use std::time::Duration;

fn main() {
    let root = std::path::PathBuf::from("/home/ichiyo/Workspace/agent-rs");
    let paths = agent_paths::AppPaths::new(
        std::env::temp_dir().join("probe/config"),
        std::env::temp_dir().join("probe/data"),
        ".husk",
    );
    let (mut mgr, rx) = husk_host::session_manager::SessionManager::spawn_at(Some(root), paths);
    eprintln!(
        "booted, active={} provider={} model={}",
        mgr.active_id, mgr.provider_name, mgr.model_name
    );
    std::thread::spawn(move || {
        while let Ok((_, id, ev)) = rx.recv() {
            let s = format!("{ev:?}");
            eprintln!("EV[{id}] {}", &s[..s.len().min(200)]);
        }
    });
    mgr.open_session(112);
    let tx = mgr.active().expect("active handle").cmd_tx.clone();
    // Match the live session: cline + deepseek-v4.1-flash + max thinking + auto.
    tx.try_send(husk_host::command::UiCommand::SetModel {
        provider: "cline(chat compat)".into(),
        model: "deepseek/deepseek-v4.1-flash".into(),
    })
    .unwrap();
    tx.try_send(husk_host::command::UiCommand::SetThinkingLevel {
        level: "max".into(),
    })
    .unwrap();
    tx.try_send(husk_host::command::UiCommand::SetPermissionMode {
        mode: "auto".into(),
    })
    .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    tx.try_send(husk_host::command::UiCommand::Prompt {
        text: "/slugify Java 实践".into(),
    })
    .unwrap();
    eprintln!("prompt sent, watching 120s...");
    for i in 0..24 {
        std::thread::sleep(Duration::from_secs(5));
        let running = mgr.active().map(|h| h.running).unwrap_or(false);
        eprintln!("t+{}s running={running}", (i + 1) * 5);
        if !running && i > 1 {
            eprintln!("turn done");
            break;
        }
    }
}
