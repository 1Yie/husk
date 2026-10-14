//! Startup failures are exercised in subprocesses so the global database
//! cache and XDG environment are isolated from other tests and live data.

#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use agent_ipc::UiCommand;
use agent_kernel::session::{SessionActor, SessionConfig};
use agent_kernel::session_manager::SessionManager;
use agent_kernel::session_store::SessionStore;

mod common;
use common::ScriptedProvider;

const PROBE_MODE: &str = "HUSK_STORAGE_BOOT_PROBE";

fn probe(dir: &std::path::Path, mode: &str) -> std::process::Output {
    probe_command(dir, mode).output().unwrap()
}

fn probe_command(dir: &std::path::Path, mode: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "storage_boot_probe", "--nocapture"])
        .env(PROBE_MODE, mode)
        .env("AGENT_SESSION_DB", dir.join("sessions.db"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CONFIG_HOME", dir.join("config"));
    cmd
}

fn assert_probe_succeeded(output: std::process::Output) {
    assert!(
        output.status.success(),
        "startup probe failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn another_process_can_read_and_write_during_empty_or_workspace_boot() {
    for mode in ["parallel-empty", "parallel-workspace"] {
        let dir = tempfile::tempdir().unwrap();
        let holder = SessionStore::open_at(dir.path(), &dir.path().join("sessions.db")).unwrap();
        assert_probe_succeeded(probe(dir.path(), mode));
        drop(holder);
        let disk = SessionStore::open_at(
            &dir.path().join("workspace"),
            &dir.path().join("sessions.db"),
        )
        .unwrap();
        assert!(
            disk.load_history(77).is_some(),
            "both processes must share durable session files"
        );
    }
}

#[test]
fn healthy_database_boot_still_persists_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    assert_probe_succeeded(probe(dir.path(), "normal"));
    let disk = SessionStore::open_at(
        &dir.path().join("workspace"),
        &dir.path().join("sessions.db"),
    )
    .unwrap();
    assert!(disk.load_history(77).is_some());
}

#[test]
fn simultaneous_processes_allocate_and_commit_without_losing_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = probe_command(dir.path(), "allocate");
    let mut b = probe_command(dir.path(), "allocate");
    use std::process::Stdio;
    a.stdout(Stdio::piped()).stderr(Stdio::piped());
    b.stdout(Stdio::piped()).stderr(Stdio::piped());
    let first = a.spawn().unwrap();
    let second = b.spawn().unwrap();
    assert_probe_succeeded(first.wait_with_output().unwrap());
    assert_probe_succeeded(second.wait_with_output().unwrap());
    let store = SessionStore::open_at(
        &dir.path().join("workspace"),
        &dir.path().join("sessions.db"),
    )
    .unwrap();
    let sessions = store.list();
    assert_eq!(sessions.len(), 80);
    for meta in sessions {
        assert_eq!(store.load_history(meta.id).unwrap().len(), 1);
    }
}

#[test]
fn invalid_database_boot_preserves_the_file_and_uses_one_temporary_store() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sessions.db");
    std::fs::write(&file, b"invalid database").unwrap();
    assert_probe_succeeded(probe(dir.path(), "invalid"));
    assert_eq!(std::fs::read(file).unwrap(), b"invalid database");
}

#[test]
fn failed_open_can_be_retried_after_the_path_is_fixed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sessions.db"), b"invalid source").unwrap();
    assert_probe_succeeded(probe(dir.path(), "retry"));
}

#[test]
fn storage_boot_probe() {
    let Ok(mode) = std::env::var(PROBE_MODE) else {
        return;
    };
    let path = PathBuf::from(std::env::var_os("AGENT_SESSION_DB").unwrap());
    let root = path.parent().unwrap().join("workspace");
    std::fs::create_dir_all(&root).unwrap();

    if mode == "allocate" {
        let store = SessionStore::open(&root).unwrap();
        for _ in 0..40 {
            let id = store.next_id();
            let meta = serde_json::from_value(
                serde_json::json!({"id":id,"title":"parallel","preview":"","updated_at":1}),
            )
            .unwrap();
            store
                .commit_turn(
                    &[agent_llm::ChatMessage::user(format!(
                        "pid={}",
                        std::process::id()
                    ))],
                    meta,
                )
                .unwrap();
        }
        return;
    }

    if mode == "retry" {
        let error = SessionStore::open(&root)
            .err()
            .expect("invalid source cannot be imported");
        assert!(
            error.to_string().contains("sessions.db"),
            "preserve actual path in error: {error}"
        );
        std::fs::remove_file(&path).unwrap();
        let store = SessionStore::open(&root).expect("failed opens must not poison future opens");
        store.write_state(1, "retry", b"ok").unwrap();
        return;
    }

    let explicit = mode == "parallel-workspace";
    let (mut manager, _) = SessionManager::spawn_at(
        explicit.then(|| root.clone()),
        agent_paths::AppPaths::new(
            std::env::temp_dir().join("husk-boot-test/config"),
            std::env::temp_dir().join("husk-boot-test/data"),
            ".husk",
        ),
    );
    let warning = agent_kernel::session_store::session_storage_warning();
    if mode != "invalid" {
        assert!(warning.is_none());
    } else {
        let warning = warning.expect("transient startup must expose its storage status");
        assert!(warning.contains("sessions.db"));
        assert!(warning.contains("退出后不会保留"));
    }
    if !explicit {
        assert!(!manager.workspace_active);
        manager.switch_workspace(root.clone()).unwrap();
    }
    assert!(manager.workspace_active);

    // Actor construction must obtain the same fallback as the manager.
    let provider = ScriptedProvider::new();
    provider.script_text("temporary reply");
    let (mut actor, _) = SessionActor::spawn_with_id(
        SessionConfig {
            workspace_root: root.clone(),
            paths: agent_paths::AppPaths::for_tests("kernel"),
            provider: Arc::new(provider),
            provider_name: "test".into(),
            model: "test".into(),
            temperature: 0.0,
            permission_mode: "default".into(),
            agent_mode: "build".into(),
            track_dirty: false,
            thinking_level: None,
            thinking_level_map: None,
            context_window: None,
            model_input: Vec::new(),
            compact_at: None,
            model_params: None,
            plugins: None,
            queued_prompts: Vec::new(),
            memory_enabled: false,
            memory_distill: false,
        },
        77,
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(actor.handle(UiCommand::Prompt {
        text: "temporary task".into(),
    }));
    assert!(
        manager.store_history(77).is_some(),
        "manager and actor must share the same database"
    );
    let store = SessionStore::open(&root).unwrap();
    store.write_state(77, "todo", b"temporary todo").unwrap();
    assert_eq!(
        SessionStore::open_existing(&root)
            .unwrap()
            .read_state(77, "todo")
            .unwrap(),
        Some(b"temporary todo".to_vec())
    );
}
