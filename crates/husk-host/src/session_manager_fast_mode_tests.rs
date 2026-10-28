use super::*;

fn fixture() -> (tempfile::TempDir, SessionManager) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let paths = AppPaths::new(
        temp.path().join("config"),
        temp.path().join("data"),
        ".husk",
    );
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(
        paths.config_dir.join("config.toml"),
        r#"
active_provider = "test"
active_model = "a"
[providers.test]
type = "openai_compat"
base_url = "https://example.invalid/v1"
api_key = "test"
models = [
  { id = "a", fastMode = { mode = "priority" } },
  { id = "b", fastMode = { mode = "speed" } },
  { id = "plain" },
  { id = "overridden" },
]
[providers.test.modelOverrides.overridden.fastMode]
mode = "custom"
params = { gateway = "fast" }
"#,
    )
    .unwrap();
    let (manager, _events) = SessionManager::spawn_at(Some(root), paths);
    (temp, manager)
}

#[test]
fn fast_capability_is_explicit_effective_and_legacy_state_defaults_off() {
    let (_temp, mut manager) = fixture();
    let info = manager.model_info();
    assert!(!info.active_fast_mode);
    for id in ["a", "b", "overridden"] {
        assert!(
            info.models
                .iter()
                .find(|m| m.model == id)
                .unwrap()
                .supports_fast_mode,
            "{id}"
        );
    }
    assert!(
        !info
            .models
            .iter()
            .find(|m| m.model == "plain")
            .unwrap()
            .supports_fast_mode
    );
    assert!(!crate::session::read_fast_mode(
        &manager.store,
        manager.active_id
    ));
}

#[test]
fn busy_actor_ack_is_durable_and_does_not_wait_for_the_command_pump() {
    let (_temp, mut manager) = fixture();
    let id = manager.active_id;
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let handle = manager.handles.get_mut(&id).unwrap();
    handle.cmd_tx = tx;
    handle.running = true;
    assert_eq!(manager.set_fast_mode(Some(id), None, true), Ok(true));
    assert!(crate::session::read_fast_mode(&manager.store, id));
    assert!(manager.model_info().active_fast_mode);
    // Full queue: never ACK a preference the actor cannot be woken for.
    assert!(manager.set_fast_mode(Some(id), None, false).is_err());
    assert!(crate::session::read_fast_mode(&manager.store, id));
    assert!(matches!(
        rx.try_recv().unwrap(),
        crate::command::UiCommand::SetFastMode { enabled: true }
    ));
}

#[test]
fn fast_toggle_reopens_forks_and_stays_session_and_workspace_scoped() {
    let (temp, mut manager) = fixture();
    let source = manager.active_id;
    let root = manager.workspace_root.to_string_lossy().into_owned();
    manager
        .store
        .snapshot(
            source,
            &[agent_llm::types::ChatMessage::user("retain source")],
        )
        .unwrap();
    assert_eq!(
        manager.set_fast_mode(Some(source), Some(&root), true),
        Ok(true)
    );
    let fork = manager.fork_session(source).unwrap();
    assert_ne!(fork, source);
    assert!(manager.model_info().active_fast_mode);
    assert!(crate::session::read_fast_mode(&manager.store, fork));
    manager.new_session();
    let fresh = manager.active_id;
    assert!(!manager.model_info().active_fast_mode);
    assert!(manager
        .set_fast_mode(Some(source), Some(&root), false)
        .is_err());
    assert!(!crate::session::read_fast_mode(&manager.store, fresh));
    manager.open_session(source);
    if let Some(handle) = manager.handles.remove(&source) {
        handle.shutdown.notify_one();
    }
    manager.open_session(source);
    assert!(
        manager.model_info().active_fast_mode,
        "reopened actor restores the durable flag"
    );
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    manager.switch_workspace(other).unwrap();
    let active = manager.active_id;
    assert!(!manager.model_info().active_fast_mode);
    assert!(manager
        .set_fast_mode(Some(active), Some(&root), true)
        .is_err());
    assert!(!crate::session::read_fast_mode(&manager.store, active));
    manager
        .switch_workspace(std::path::PathBuf::from(root))
        .unwrap();
    manager.open_session(source);
    assert!(manager.model_info().active_fast_mode);
}

#[test]
fn enabled_fast_mode_keeps_an_empty_session_reopenable() {
    let (_temp, mut manager) = fixture();
    let source = manager.active_id;
    assert_eq!(manager.set_fast_mode(Some(source), None, true), Ok(true));
    manager.new_session();
    assert!(!manager.model_info().active_fast_mode);
    assert!(manager.store.meta(source).is_some());
    manager.open_session(source);
    assert!(manager.model_info().active_fast_mode);
}

#[test]
fn manager_model_and_config_changes_normalize_without_reenabling() {
    let (_temp, mut manager) = fixture();
    assert_eq!(manager.set_fast_mode(None, None, true), Ok(true));
    manager.set_model("test".into(), "b".into());
    assert!(*manager.active().unwrap().fast_mode.read().unwrap());
    manager.set_model("test".into(), "plain".into());
    assert!(!*manager.active().unwrap().fast_mode.read().unwrap());
    assert!(!crate::session::read_fast_mode(
        &manager.store,
        manager.active_id
    ));
    manager.set_model("test".into(), "a".into());
    assert!(!manager.model_info().active_fast_mode);
    assert_eq!(manager.set_fast_mode(None, None, true), Ok(true));
    let mut cfg = manager.provider_cfg.clone();
    cfg.providers.clear();
    manager.apply_loaded_config(cfg);
    assert!(!*manager.active().unwrap().fast_mode.read().unwrap());
    assert!(!crate::session::read_fast_mode(
        &manager.store,
        manager.active_id
    ));
}
