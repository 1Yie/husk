use super::*;
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, agent_paths::AppPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths =
        agent_paths::AppPaths::new(dir.path().join("config"), dir.path().join("data"), ".husk");
    (dir, paths)
}

#[test]
fn office_workspace_can_be_created_lazily_without_creating_arbitrary_paths() {
    if isolated("office_workspace_can_be_created_lazily_without_creating_arbitrary_paths") {
        return;
    }
    let (dir, mut paths) = fixture();
    // The shared store deliberately uses one process-local directory in test
    // binaries, independent of the data directory passed to the host.
    paths.data_dir = agent_store::app_data_dir().unwrap();
    let office = paths.data_dir.join("office");
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let missing = dir.path().join("not-selected-by-the-user");
    let request = bridge
        .send(Command::SwitchWorkspace(missing.clone()))
        .unwrap();
    assert!(error_for(&events, request).contains("existing directory"));
    assert!(!missing.exists());
    let request = bridge
        .send(Command::SwitchWorkspace(office.clone()))
        .unwrap();
    let state = snapshot(&events, request);
    assert!(office.is_dir());
    assert_eq!(
        state.active.unwrap().root,
        office.canonicalize().unwrap().to_string_lossy()
    );
}

fn isolated(name: &str) -> bool {
    if std::env::var("HUSK_BRIDGE_TEST").as_deref() == Ok(name) {
        return false;
    }
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("bridge::tests::{name}"), "--nocapture"])
        .env("HUSK_BRIDGE_TEST", name)
        .env("TMPDIR", dir.path())
        .env("TMP", dir.path())
        .env("TEMP", dir.path())
        .env("AGENT_SESSION_DB", dir.path().join("sessions"))
        .status()
        .unwrap();
    assert!(status.success(), "isolated test {name} failed");
    true
}

fn snapshot(events: &async_channel::Receiver<BridgeEvent>, id: u64) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match events.try_recv() {
            Ok(BridgeEvent::Snapshot {
                request_id,
                snapshot,
            }) if request_id == id => {
                return snapshot;
            }
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => {
                panic!("{message}");
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "snapshot #{id} timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn boot_is_nonblocking_and_uses_supplied_paths() {
    if isolated("boot_is_nonblocking_and_uses_supplied_paths") {
        return;
    }
    let (_dir, paths) = fixture();
    let start = Instant::now();
    let bridge = HostBridge::spawn(paths.clone()).unwrap();
    assert!(start.elapsed() < Duration::from_millis(250));
    let state = snapshot(&bridge.events(), 0);
    assert_eq!(state.config.config_dir, paths.config_dir);
    assert_eq!(state.config.data_dir, paths.data_dir);
    let start = Instant::now();
    bridge.shutdown();
    assert!(start.elapsed() < Duration::from_millis(250));
    assert!(bridge.send(Command::Refresh).is_err());
}

fn error_for(events: &async_channel::Receiver<BridgeEvent>, id: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match events.try_recv() {
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => return message,
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "error #{id} timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn workspace_identity_and_stale_approvals_are_not_active_session_aliases() {
    if isolated("workspace_identity_and_stale_approvals_are_not_active_session_aliases") {
        return;
    }
    let (dir, paths) = fixture();
    let a = dir.path().join("alpha");
    let b = dir.path().join("beta");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(a.clone())).unwrap();
    let first = snapshot(&events, id);
    let key_a = first.active.unwrap();
    assert_eq!(key_a.root, a.canonicalize().unwrap().to_string_lossy());
    assert!(first.history_loaded);
    let id = bridge.send(Command::SwitchWorkspace(b.clone())).unwrap();
    let second = snapshot(&events, id);
    let key_b = second.active.unwrap();
    assert_eq!(key_b.root, b.canonicalize().unwrap().to_string_lossy());
    assert_ne!(key_a, key_b);
    let forged = SessionKey {
        root: dir.path().join("unopened").to_string_lossy().into(),
        id: key_b.id,
    };
    let id = bridge
        .send(Command::Send {
            key: forged,
            text: "/help".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("no longer live"));
    let id = bridge
        .send(Command::Decide {
            key: key_b.clone(),
            request_id: 9911,
            approved: true,
            action: None,
            child_id: Some(7),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("stale"));
    let id = bridge.send(Command::Refresh).unwrap();
    let refreshed = snapshot(&events, id);
    assert!(!refreshed.history_loaded);
    assert!(refreshed.history.is_empty());
    assert!(refreshed.event_sequence >= second.event_sequence);
    let id = bridge
        .send(Command::SetPermissionMode {
            key: key_b.clone(),
            mode: "bypassPermissions".into(),
        })
        .unwrap();
    let configured = snapshot(&events, id);
    assert_eq!(configured.config.permission_mode, "bypassPermissions");
    assert!(!configured.config.computer_control_enabled);
    assert!(!configured.history_loaded);
    let id = bridge
        .send(Command::SetPermissionMode {
            key: key_b.clone(),
            mode: "not-a-mode".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("unknown permission mode"));
    let id = bridge.send(Command::Refresh).unwrap();
    assert_eq!(
        snapshot(&events, id).config.permission_mode,
        "bypassPermissions"
    );
    let id = bridge
        .send(Command::SetPermissionMode {
            key: key_a,
            mode: "default".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("no longer active"));
    let id = bridge.send(Command::Refresh).unwrap();
    assert_eq!(
        snapshot(&events, id).config.permission_mode,
        "bypassPermissions"
    );
}

#[test]
fn model_and_session_settings_are_reported_by_the_real_host() {
    if isolated("model_and_session_settings_are_reported_by_the_real_host") {
        return;
    }
    let (dir, paths) = fixture();
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(
        paths.config_dir.join("config.json"),
        serde_json::to_vec(&serde_json::json!({
            "active_provider": "local",
            "active_model": "first",
            "providers": {
                "local": {
                    "type": "openai_completions",
                    "base_url": "http://127.0.0.1:1/v1",
                    "api_key": "test-only",
                    "models": [
                        {"id": "first", "name": "First"},
                        {
                            "id": "second",
                            "name": "Second",
                            "reasoning": true,
                            "fastMode": {"mode": "priority"},
                            "thinking_level_map": {"low": "low", "high": "high"}
                        }
                    ]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let root = dir.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(root)).unwrap();
    let initial = snapshot(&events, id);
    let key = initial.active.unwrap();
    assert!(
        initial.model_info["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["provider"] == "local" && model["model"] == "second")
    );

    let id = bridge
        .send(Command::SetModel {
            key: key.clone(),
            provider: "local".into(),
            model: "second".into(),
        })
        .unwrap();
    let selected = snapshot(&events, id);
    assert_eq!(selected.model_info["active_provider"], "local");
    assert_eq!(selected.model_info["active_model"], "second");
    assert!(
        selected.model_info["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["model"] == "second" && model["supports_fast_mode"] == true)
    );
    assert!(!selected.history_loaded);

    let id = bridge
        .send(Command::SetFastMode {
            key: key.clone(),
            enabled: true,
        })
        .unwrap();
    assert_eq!(snapshot(&events, id).model_info["active_fast_mode"], true);

    let id = bridge
        .send(Command::SetThinkingLevel {
            key: key.clone(),
            level: "high".into(),
        })
        .unwrap();
    let thinking = snapshot(&events, id);
    assert_eq!(thinking.config.thinking_level.as_deref(), Some("high"));
    assert_eq!(thinking.model_info["active_thinking_level"], "high");

    let id = bridge
        .send(Command::SetAgentMode {
            key: key.clone(),
            mode: "plan".into(),
        })
        .unwrap();
    let planned = snapshot(&events, id);
    assert_eq!(planned.config.agent_mode, "plan");
    assert_eq!(planned.model_info["active_agent_mode"], "plan");
    assert_eq!(planned.active.as_ref(), Some(&key));

    let id = bridge
        .send(Command::SetModel {
            key: key.clone(),
            provider: "local".into(),
            model: "missing".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("unknown model"));
    let id = bridge
        .send(Command::SetAgentMode {
            key: key.clone(),
            mode: "missing".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("unknown agent mode"));
    let id = bridge.send(Command::Refresh).unwrap();
    let unchanged = snapshot(&events, id);
    assert_eq!(unchanged.model_info["active_model"], "second");
    assert_eq!(unchanged.config.agent_mode, "plan");
    assert_eq!(unchanged.model_info["active_fast_mode"], true);

    let id = bridge.send(Command::NewSession).unwrap();
    let other_snapshot = snapshot(&events, id);
    let other = other_snapshot.active.clone().unwrap();
    assert_ne!(other, key);
    let id = bridge
        .send(Command::SetFastMode {
            key: key.clone(),
            enabled: false,
        })
        .unwrap();
    assert!(error_for(&events, id).contains("no longer the active session"));
    for command in [
        Command::SetModel {
            key: key.clone(),
            provider: "local".into(),
            model: "second".into(),
        },
        Command::SetThinkingLevel {
            key: key.clone(),
            level: "high".into(),
        },
        Command::SetPermissionMode {
            key: key.clone(),
            mode: "bypassPermissions".into(),
        },
        Command::SetAgentMode {
            key: key.clone(),
            mode: "plan".into(),
        },
    ] {
        let id = bridge.send(command).unwrap();
        assert!(error_for(&events, id).contains("no longer active"));
    }
    let id = bridge.send(Command::Refresh).unwrap();
    let still_other = snapshot(&events, id);
    assert_eq!(still_other.active.as_ref(), Some(&other));
    assert_eq!(
        still_other.model_info["active_model"],
        other_snapshot.model_info["active_model"]
    );
    assert_eq!(
        still_other.config.permission_mode,
        other_snapshot.config.permission_mode
    );
    assert_eq!(
        still_other.config.agent_mode,
        other_snapshot.config.agent_mode
    );
    let id = bridge.send(Command::OpenSession(key.clone())).unwrap();
    assert_eq!(snapshot(&events, id).model_info["active_fast_mode"], true);
    let id = bridge
        .send(Command::SetFastMode {
            key,
            enabled: false,
        })
        .unwrap();
    assert_eq!(snapshot(&events, id).model_info["active_fast_mode"], false);
}

#[test]
fn appearance_mode_persists_without_overwriting_other_preferences() {
    if isolated("appearance_mode_persists_without_overwriting_other_preferences") {
        return;
    }
    let (_dir, paths) = fixture();
    let bridge = HostBridge::spawn(paths.clone()).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let mut appearance = husk_host::prefs::load_appearance_settings();
    appearance.ui_font = "preserved-font".into();
    husk_host::prefs::save_appearance_settings(&appearance).unwrap();

    let id = bridge.send(Command::SetThemeMode("dark".into())).unwrap();
    assert_eq!(snapshot(&events, id).config.theme_mode, "dark");
    let saved = husk_host::prefs::load_appearance_settings();
    assert_eq!(saved.theme_mode, "dark");
    assert_eq!(saved.ui_font, "preserved-font");

    let id = bridge
        .send(Command::SetThemeMode("invalid".into()))
        .unwrap();
    assert!(error_for(&events, id).contains("unknown theme mode"));
    let id = bridge.send(Command::Refresh).unwrap();
    assert_eq!(snapshot(&events, id).config.theme_mode, "dark");

    let id = bridge.send(Command::SetThemeMode("system".into())).unwrap();
    assert_eq!(snapshot(&events, id).config.theme_mode, "system");

    let id = bridge.send(Command::SetThemeMode("dark".into())).unwrap();
    snapshot(&events, id);
    bridge.shutdown();
    drop(bridge);
    let deadline = Instant::now() + Duration::from_secs(5);
    let reopened = loop {
        match HostBridge::spawn(paths.clone()) {
            Ok(bridge) => break bridge,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                assert!(Instant::now() < deadline, "previous host did not shut down");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    };
    assert_eq!(snapshot(&reopened.events(), 0).config.theme_mode, "dark");
}

#[test]
fn desktop_notification_preference_fails_closed_on_corrupt_storage() {
    if isolated("desktop_notification_preference_fails_closed_on_corrupt_storage") {
        return;
    }
    let (_dir, paths) = fixture();
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    assert_eq!(
        snapshot(&events, 0).config.desktop_notifications,
        Some(true)
    );

    let mut prefs = husk_host::prefs::load_default_preferences();
    prefs.compact_at = 0.7;
    husk_host::prefs::save_default_preferences(&prefs).unwrap();
    let id = bridge
        .send(Command::SetDesktopNotifications(false))
        .unwrap();
    assert_eq!(
        snapshot(&events, id).config.desktop_notifications,
        Some(false)
    );
    let saved = husk_host::prefs::load_default_preferences();
    assert!(!saved.desktop_notifications);
    assert_eq!(saved.compact_at, 0.7);

    let pref_path = agent_store::migrate_session_database()
        .unwrap()
        .join("globals/default_preferences.json");
    std::fs::write(&pref_path, b"{invalid").unwrap();
    let id = bridge.send(Command::Refresh).unwrap();
    assert_eq!(snapshot(&events, id).config.desktop_notifications, None);
    let id = bridge.send(Command::SetDesktopNotifications(true)).unwrap();
    let _error = error_for(&events, id);
    assert_eq!(std::fs::read(pref_path).unwrap(), b"{invalid");
}

#[test]
fn memory_preferences_are_persisted_without_overwriting_other_defaults() {
    if isolated("memory_preferences_are_persisted_without_overwriting_other_defaults") {
        return;
    }
    let (_dir, paths) = fixture();
    let bridge = HostBridge::spawn(paths.clone()).unwrap();
    let events = bridge.events();
    let state = snapshot(&events, 0);
    assert_eq!(
        state.config.memory_preferences.unwrap(),
        husk_host::prefs::memory_preferences().unwrap()
    );
    let mut original = husk_host::prefs::load_default_preferences();
    original.compact_at = 0.7;
    original.desktop_notifications = false;
    original.permission_mode = "dontAsk".into();
    husk_host::prefs::save_default_preferences(&original).unwrap();
    let id = bridge.send(Command::SetMemoryEnabled(false)).unwrap();
    let state = snapshot(&events, id);
    let memory = state.config.memory_preferences.unwrap();
    assert!(!memory.enabled);
    assert_eq!(memory.distill, original.memory_distill);
    let id = bridge.send(Command::SetMemoryDistill(false)).unwrap();
    let memory = snapshot(&events, id).config.memory_preferences.unwrap();
    assert!(!memory.enabled && !memory.distill);
    let saved = husk_host::prefs::load_default_preferences();
    assert_eq!(saved.compact_at, original.compact_at);
    assert_eq!(saved.permission_mode, original.permission_mode);
    assert!(!saved.desktop_notifications);
    assert_eq!(saved.sandbox_enabled, original.sandbox_enabled);
    assert_eq!(saved.semantic_recall, original.semantic_recall);

    bridge.shutdown();
    drop(bridge);
    let deadline = Instant::now() + Duration::from_secs(5);
    let reopened = loop {
        match HostBridge::spawn(paths.clone()) {
            Ok(bridge) => break bridge,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                assert!(Instant::now() < deadline, "previous host did not shut down");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    };
    assert_eq!(
        snapshot(&reopened.events(), 0).config.memory_preferences,
        Some(memory)
    );
}

#[test]
fn corrupt_memory_preferences_do_not_change_stored_data_or_host_defaults() {
    if isolated("corrupt_memory_preferences_do_not_change_stored_data_or_host_defaults") {
        return;
    }
    let (_dir, paths) = fixture();
    husk_host::office::init_store(paths.data_dir.clone());
    let (mut manager, _events) = husk_host::session_manager::SessionManager::spawn(paths);
    manager
        .set_memory_prefs_checked(Some(false), Some(false))
        .unwrap();
    assert_eq!(manager.memory_prefs(), (false, false));
    let pref_path = agent_store::migrate_session_database()
        .unwrap()
        .join("globals/default_preferences.json");
    std::fs::write(&pref_path, b"{invalid").unwrap();
    assert!(husk_host::prefs::memory_preferences().is_err());
    assert!(
        manager
            .set_memory_prefs_checked(Some(true), Some(true))
            .is_err()
    );
    assert_eq!(manager.memory_prefs(), (false, false));
    assert_eq!(std::fs::read(pref_path).unwrap(), b"{invalid");
}

#[test]
fn real_host_events_keep_their_session_and_refresh_does_not_reset_history() {
    if isolated("real_host_events_keep_their_session_and_refresh_does_not_reset_history") {
        return;
    }
    let (dir, paths) = fixture();
    let root = dir.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(root)).unwrap();
    let key = snapshot(&events, id).active.unwrap();
    bridge
        .send(Command::Send {
            key: key.clone(),
            text: "/help".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match events.try_recv() {
            Ok(BridgeEvent::Ui {
                key: routed,
                event: agent_ipc::UiEvent::SystemMessage(text),
            }) => {
                assert_eq!(routed, key);
                assert!(!text.is_empty());
                break;
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("{e}"),
        }
    }
    let id = bridge.send(Command::Refresh).unwrap();
    let refreshed = snapshot(&events, id);
    assert!(!refreshed.history_loaded);
    assert!(refreshed.history.is_empty());
    assert!(refreshed.event_sequence > 0);
    let id = bridge
        .send(Command::AnswerQuestion {
            key,
            request_id: 999,
            answer: "stale".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("no longer pending"));
}

#[test]
fn restored_queues_remain_scoped_and_stale_removal_is_rejected() {
    if isolated("restored_queues_remain_scoped_and_stale_removal_is_rejected") {
        return;
    }
    let (dir, paths) = fixture();
    husk_host::office::init_store(paths.data_dir.clone());
    let a = dir.path().join("queue-a");
    let b = dir.path().join("queue-b");
    for (root, item) in [(&a, "alpha"), (&b, "beta")] {
        std::fs::create_dir_all(root).unwrap();
        let store = agent_store::SessionStore::open(root).unwrap();
        store
            .commit_turn(
                &[agent_llm::types::ChatMessage::user("history")],
                agent_store::SessionMeta {
                    id: 1,
                    title: "session".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        store.update_queue(1, vec![item.into()]).unwrap();
        store.set_last_active(1).unwrap();
    }
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(a.clone())).unwrap();
    let a_state = snapshot(&events, id);
    let a_key = a_state.active.unwrap();
    assert_eq!(a_state.queued_prompts, ["alpha"]);
    let id = bridge.send(Command::SwitchWorkspace(b.clone())).unwrap();
    let b_state = snapshot(&events, id);
    assert_eq!(b_state.queued_prompts, ["beta"]);

    let id = bridge
        .send(Command::RemoveQueued {
            key: a_key.clone(),
            index: 0,
            expected: vec!["stale".into()],
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut refreshed = false;
    let mut rejected = false;
    while !refreshed || !rejected {
        match events.try_recv() {
            Ok(BridgeEvent::Ui {
                key,
                event: agent_ipc::UiEvent::QueuedPrompts { items },
            }) if key == a_key && items == ["alpha"] => refreshed = true,
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => {
                assert!(message.contains("队列已变化"));
                rejected = true;
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "queue refresh timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
    let id = bridge
        .send(Command::RemoveQueued {
            key: a_key.clone(),
            index: 0,
            expected: vec!["alpha".into()],
        })
        .unwrap();
    let b_state = snapshot(&events, id);
    assert_eq!(b_state.queued_prompts, ["beta"]);
    assert!(
        agent_store::SessionStore::open_existing(&a)
            .unwrap()
            .meta(a_key.id)
            .unwrap()
            .queued_prompts
            .is_empty()
    );
    let id = bridge.send(Command::SwitchWorkspace(a)).unwrap();
    assert!(snapshot(&events, id).queued_prompts.is_empty());
}

#[test]
fn restored_queue_can_be_resumed_without_sending_a_new_message() {
    if isolated("restored_queue_can_be_resumed_without_sending_a_new_message") {
        return;
    }
    let (dir, paths) = fixture();
    husk_host::office::init_store(paths.data_dir.clone());
    let root = dir.path().join("resume-queue");
    std::fs::create_dir_all(&root).unwrap();
    let store = agent_store::SessionStore::open(&root).unwrap();
    store
        .commit_turn(
            &[agent_llm::types::ChatMessage::user("history")],
            agent_store::SessionMeta {
                id: 1,
                title: "session".into(),
                ..Default::default()
            },
        )
        .unwrap();
    store.update_queue(1, vec!["/help".into()]).unwrap();
    store.set_last_active(1).unwrap();
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(root)).unwrap();
    let state = snapshot(&events, id);
    assert_eq!(state.queued_prompts, ["/help"]);
    let key = state.active.unwrap();
    let id = bridge
        .send(Command::ResumeQueued { key: key.clone() })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut snapshot_seen = false;
    let mut prompt_ran = false;
    while !snapshot_seen || !prompt_ran {
        match events.try_recv() {
            Ok(BridgeEvent::Snapshot { request_id, .. }) if request_id == id => {
                snapshot_seen = true;
            }
            Ok(BridgeEvent::Ui {
                key: owner,
                event: agent_ipc::UiEvent::SystemMessage(text),
            }) if owner == key && !text.is_empty() => prompt_ran = true,
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => panic!("{message}"),
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "queued prompt did not run");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
    assert!(store.meta(1).unwrap().queued_prompts.is_empty());
}

#[test]
fn queue_edits_and_moves_persist_without_rewriting_history() {
    if isolated("queue_edits_and_moves_persist_without_rewriting_history") {
        return;
    }
    let (dir, paths) = fixture();
    husk_host::office::init_store(paths.data_dir.clone());
    let root = dir.path().join("edit-queue");
    std::fs::create_dir_all(&root).unwrap();
    let store = agent_store::SessionStore::open(&root).unwrap();
    let history = vec![agent_llm::types::ChatMessage::user("existing history")];
    store
        .commit_turn(
            &history,
            agent_store::SessionMeta {
                id: 1,
                title: "session".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let original = vec!["first".to_owned(), "second".to_owned(), "third".to_owned()];
    store.update_queue(1, original.clone()).unwrap();
    store.set_last_active(1).unwrap();
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(root)).unwrap();
    let key = snapshot(&events, id).active.unwrap();

    let updated = |id| {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut accepted = false;
        let mut state = None;
        loop {
            match events.try_recv() {
                Ok(BridgeEvent::QueueUpdated { request_id }) if request_id == id => {
                    accepted = true;
                }
                Ok(BridgeEvent::Snapshot {
                    request_id,
                    snapshot,
                }) if request_id == id => {
                    state = Some(snapshot);
                }
                Ok(BridgeEvent::Error {
                    request_id,
                    message,
                }) if request_id == id => {
                    panic!("{message}");
                }
                Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "queue update timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
            if accepted && let Some(state) = state.take() {
                break state;
            }
        }
    };
    let id = bridge
        .send(Command::MoveQueued {
            key: key.clone(),
            from: 2,
            to: 0,
            expected: original.clone(),
        })
        .unwrap();
    let moved = updated(id).queued_prompts;
    assert_eq!(moved, ["third", "first", "second"]);
    let edited = "edited\n\n<attached-image path=\"/not-read.png\" name=\"kept.png\"/>";
    let id = bridge
        .send(Command::EditQueued {
            key: key.clone(),
            index: 1,
            expected: moved,
            text: edited.into(),
        })
        .unwrap();
    let state = updated(id);
    assert_eq!(state.queued_prompts, ["third", edited, "second"]);
    assert_eq!(
        store.meta(key.id).unwrap().queued_prompts,
        state.queued_prompts
    );
    let id = bridge
        .send(Command::MoveQueued {
            key: key.clone(),
            from: 0,
            to: 2,
            expected: original.clone(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("队列已变化"));
    let id = bridge
        .send(Command::EditQueued {
            key: key.clone(),
            index: 0,
            expected: original,
            text: "must not replace the newer item".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("队列已变化"));
    let id = bridge.send(Command::Refresh).unwrap();
    assert_eq!(snapshot(&events, id).queued_prompts, state.queued_prompts);
    assert_eq!(store.load_history(key.id).unwrap(), history);
}

#[test]
fn running_turn_queues_and_persists_followups_before_it_finishes() {
    if isolated("running_turn_queues_and_persists_followups_before_it_finishes") {
        return;
    }
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    let (dir, paths) = fixture();
    let root = dir.path().join("queue-running");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::write(
        paths.config_dir.join("config.json"),
        serde_json::to_vec(&serde_json::json!({
            "active_provider": "local",
            "active_model": "test-model",
            "providers": {
                "local": {
                    "type": "openai_completions",
                    "base_url": format!("http://{address}/v1"),
                    "api_key": "test-only",
                    "models": [{"id": "test-model", "name": "Test"}]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "model request timed out");
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            let (header_end, length) = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0 && request.len() < 2 * 1024 * 1024);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let end = end + 4;
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + length {
                        break (end, length);
                    }
                }
            };
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
            let streaming = body["stream"].as_bool().unwrap_or(false);
            let response = if streaming {
                accepted_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(20)).unwrap();
                concat!(
                    "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",",
                    "\"model\":\"test-model\",\"choices\":[{\"index\":0,",
                    "\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",",
                    "\"model\":\"test-model\",\"choices\":[{\"index\":0,",
                    "\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                )
                .to_owned()
            } else {
                r#"{"id":"title","object":"chat.completion","model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"title"},"finish_reason":"stop"}]}"#.to_owned()
            };
            let content_type = if streaming {
                "text/event-stream"
            } else {
                "application/json"
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
            if streaming {
                break;
            }
        }
    });

    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let id = bridge.send(Command::SwitchWorkspace(root.clone())).unwrap();
    let key = snapshot(&events, id).active.unwrap();
    let id = bridge
        .send(Command::Send {
            key: key.clone(),
            text: "first task".into(),
        })
        .unwrap();
    snapshot(&events, id);
    accepted_rx.recv_timeout(Duration::from_secs(20)).unwrap();
    let id = bridge
        .send(Command::Send {
            key: key.clone(),
            text: "follow-up".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut snapshot_seen = false;
    let mut event_seen = false;
    while !snapshot_seen || !event_seen {
        match events.try_recv() {
            Ok(BridgeEvent::Snapshot {
                request_id,
                snapshot,
            }) if request_id == id => {
                assert_eq!(snapshot.queued_prompts, ["follow-up"]);
                snapshot_seen = true;
            }
            Ok(BridgeEvent::Ui {
                key: owner,
                event: agent_ipc::UiEvent::QueuedPrompts { items },
            }) if owner == key && items == ["follow-up"] => event_seen = true,
            Ok(BridgeEvent::Error {
                request_id,
                message,
            }) if request_id == id => panic!("{message}"),
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "follow-up queue timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    }
    let store = agent_store::SessionStore::open_existing(&root).unwrap();
    assert_eq!(store.meta(key.id).unwrap().queued_prompts, ["follow-up"]);
    let id = bridge
        .send(Command::RemoveQueued {
            key: key.clone(),
            index: 0,
            expected: vec!["follow-up".into()],
        })
        .unwrap();
    assert!(snapshot(&events, id).queued_prompts.is_empty());
    assert!(store.meta(key.id).unwrap().queued_prompts.is_empty());
    release_tx.send(()).unwrap();
    server.join().unwrap();
    bridge.shutdown();
}

#[cfg(unix)]
#[test]
fn shutdown_and_control_errors_do_not_wait_for_boot_io() {
    if isolated("shutdown_and_control_errors_do_not_wait_for_boot_io") {
        return;
    }
    let (_dir, paths) = fixture();
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    let fifo = paths.config_dir.join("config.toml");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (opened_tx, opened_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let mut writer = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
        opened_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        writer.write_all(b"").unwrap();
    });
    let bridge = HostBridge::spawn(paths).unwrap();
    opened_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("host has opened the blocking config");
    let events = bridge.events();
    let start = Instant::now();
    let id = bridge
        .send(Command::Decide {
            key: SessionKey {
                root: "/missing".into(),
                id: 1,
            },
            request_id: 1,
            approved: false,
            action: None,
            child_id: None,
        })
        .unwrap();
    assert!(error_for(&events, id).contains("stale"));
    assert!(start.elapsed() < Duration::from_secs(2));
    let start = Instant::now();
    bridge.shutdown();
    assert!(start.elapsed() < Duration::from_millis(250));
    release_tx.send(()).unwrap();
    writer.join().unwrap();
}

#[test]
fn persisted_history_is_loaded_once_per_full_session_key() {
    if isolated("persisted_history_is_loaded_once_per_full_session_key") {
        return;
    }
    let (dir, paths) = fixture();
    husk_host::office::init_store(paths.data_dir.clone());
    let a = dir.path().join("history-a");
    let b = dir.path().join("history-b");
    for (root, text) in [(&a, "alpha history"), (&b, "beta history")] {
        std::fs::create_dir_all(root).unwrap();
        let store = agent_store::SessionStore::open(root).unwrap();
        store
            .commit_turn(
                &[agent_llm::types::ChatMessage::user(text)],
                agent_store::SessionMeta {
                    id: 1,
                    title: text.into(),
                    preview: text.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .commit_turn(
                &[agent_llm::types::ChatMessage::user("archived history")],
                agent_store::SessionMeta {
                    id: 2,
                    title: "archived history".into(),
                    archived: true,
                    ..Default::default()
                },
            )
            .unwrap();
        store.set_last_active(1).unwrap();
    }
    let bridge = HostBridge::spawn(paths).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    for (root, text) in [(&a, "alpha history"), (&b, "beta history")] {
        let id = bridge.send(Command::SwitchWorkspace(root.clone())).unwrap();
        let state = snapshot(&events, id);
        assert_eq!(state.active.as_ref().unwrap().id, 1);
        assert!(!state.sessions.iter().any(|row| row.key.root
            == root.canonicalize().unwrap().to_string_lossy()
            && row.key.id == 2));
        assert!(state.history_loaded);
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.history[0].content.as_deref(), Some(text));
    }
    let key_a = SessionKey {
        root: a.canonicalize().unwrap().to_string_lossy().into_owned(),
        id: 1,
    };
    let key_b = SessionKey {
        root: b.canonicalize().unwrap().to_string_lossy().into_owned(),
        id: 1,
    };
    for global in [false, true] {
        let id = bridge
            .send(Command::SetSessionPinned {
                key: key_a.clone(),
                pinned: true,
                global,
            })
            .unwrap();
        let state = snapshot(&events, id);
        assert_eq!(state.active.as_ref(), Some(&key_b));
        let row = state.sessions.iter().find(|row| row.key == key_a).unwrap();
        assert!(row.pinned);
        assert_eq!(row.pinned_global, global);
    }
    let id = bridge
        .send(Command::RenameSession {
            key: key_a.clone(),
            title: "  自定义标题  ".into(),
        })
        .unwrap();
    let state = snapshot(&events, id);
    assert_eq!(state.active.as_ref(), Some(&key_b));
    assert_eq!(
        state
            .sessions
            .iter()
            .find(|row| row.key == key_a)
            .unwrap()
            .title,
        "自定义标题"
    );
    let store = agent_store::SessionStore::open_existing(&a).unwrap();
    assert!(store.meta(1).unwrap().renamed);
    let id = bridge
        .send(Command::RenameSession {
            key: key_a.clone(),
            title: "  ".into(),
        })
        .unwrap();
    let state = snapshot(&events, id);
    assert_eq!(state.active.as_ref(), Some(&key_b));
    assert!(!store.meta(1).unwrap().renamed);
    let id = bridge.send(Command::DeleteSession(key_a.clone())).unwrap();
    assert!(error_for(&events, id).contains("switch to the session's workspace"));
    assert!(store.meta(1).is_some());
    let id = bridge.send(Command::Refresh).unwrap();
    assert_eq!(snapshot(&events, id).active.as_ref(), Some(&key_b));
    let id = bridge
        .send(Command::SetSessionPinned {
            key: SessionKey {
                root: dir.path().join("missing").to_string_lossy().into_owned(),
                id: 1,
            },
            pinned: true,
            global: false,
        })
        .unwrap();
    assert!(error_for(&events, id).contains("does not exist"));
    assert!(!dir.path().join("missing").exists());
    let id = bridge.send(Command::OpenSession(key_a.clone())).unwrap();
    let state = snapshot(&events, id);
    assert!(!state.history_loaded);
    assert!(state.history.is_empty());
    let row = state.sessions.iter().find(|row| row.key == key_a).unwrap();
    assert!(row.pinned && row.pinned_global);
    for global in [true, false] {
        let id = bridge
            .send(Command::SetSessionPinned {
                key: key_a.clone(),
                pinned: false,
                global,
            })
            .unwrap();
        let state = snapshot(&events, id);
        let row = state.sessions.iter().find(|row| row.key == key_a).unwrap();
        assert!(!row.pinned_global);
        assert_eq!(row.pinned, global);
    }
    // A directory at the job-file path makes cleanup fail after the tombstone.
    // The host must still stop and unregister the deleted session.
    let job_path = agent_store::migrate_session_database()
        .unwrap()
        .join(agent_store::workspace_key(&a))
        .join("1.bg.json");
    std::fs::create_dir_all(&job_path).unwrap();
    let id = bridge.send(Command::DeleteSession(key_a.clone())).unwrap();
    let state = snapshot(&events, id);
    assert!(!state.sessions.iter().any(|row| row.key == key_a));
    assert_ne!(state.active.as_ref(), Some(&key_a));
    assert!(store.meta(1).is_none());
    assert!(store.is_deleted_checked(1).unwrap());
    let id = bridge
        .send(Command::Send {
            key: key_a.clone(),
            text: "must not reach a deleted session".into(),
        })
        .unwrap();
    assert!(error_for(&events, id).contains("no longer live"));
    assert!(
        agent_store::SessionStore::open_existing(&b)
            .unwrap()
            .meta(1)
            .is_some()
    );
    let id = bridge.send(Command::DeleteSession(key_a)).unwrap();
    assert!(error_for(&events, id).contains("does not exist"));
}

#[test]
fn settings_provider_model_edits_are_persisted_and_secrets_stay_on_owner() {
    if isolated("settings_provider_model_edits_are_persisted_and_secrets_stay_on_owner") {
        return;
    }
    let (_dir, paths) = fixture();
    let bridge = HostBridge::spawn(paths.clone()).unwrap();
    let events = bridge.events();
    let first = snapshot(&events, 0);
    assert!(first.model_info["settings"].is_object());
    let request = bridge.send(Command::Settings(SettingsCommand::SaveProvider {
        original:None, id:"demo".into(), value:serde_json::json!({"kind":"openai_completions","base_url":"https://example.invalid/v1","api_key":"test-only-not-a-secret","models":[]}),
    })).unwrap();
    let saved = snapshot(&events, request);
    let masked = &saved.model_info["settings"]["config"]["value"]["providers"]["demo"];
    assert_eq!(masked["api_key"], "••••••••");
    assert!(
        !saved
            .model_info
            .to_string()
            .contains("test-only-not-a-secret")
    );
    let mut edited = masked.clone();
    edited["base_url"] = serde_json::json!("https://example.invalid/edited");
    edited["api_key"] = serde_json::json!("");
    let request = bridge
        .send(Command::Settings(SettingsCommand::SaveProvider {
            original: Some("demo".into()),
            id: "renamed".into(),
            value: edited,
        }))
        .unwrap();
    snapshot(&events, request);
    let config = agent_llm::AppConfig::load(&paths.config_dir, None).unwrap();
    assert!(!config.providers.contains_key("demo"));
    assert_eq!(
        config.providers["renamed"].api_key,
        "test-only-not-a-secret"
    );
    let request=bridge.send(Command::Settings(SettingsCommand::SaveModel{provider:"renamed".into(),original:None,value:serde_json::json!({"id":"demo-model","name":"Demo","context_window":32000,"max_tokens":4096})})).unwrap();
    snapshot(&events, request);
    assert!(
        agent_llm::AppConfig::load(&paths.config_dir, None)
            .unwrap()
            .providers["renamed"]
            .find_model("demo-model")
            .is_some()
    );
    let request = bridge
        .send(Command::Settings(SettingsCommand::SaveModel {
            provider: "renamed".into(),
            original: None,
            value: serde_json::json!({"id":"demo-model"}),
        }))
        .unwrap();
    assert!(error_for(&events, request).contains("已存在"));
    let request = bridge
        .send(Command::Settings(SettingsCommand::DeleteModel {
            provider: "renamed".into(),
            id: "demo-model".into(),
        }))
        .unwrap();
    snapshot(&events, request);
    let request = bridge
        .send(Command::Settings(SettingsCommand::DeleteProvider(
            "renamed".into(),
        )))
        .unwrap();
    let removed = snapshot(&events, request);
    assert!(
        removed.model_info["settings"]["config"]["value"]["providers"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn settings_do_not_replace_broken_config_and_invalid_paths_are_rejected() {
    if isolated("settings_do_not_replace_broken_config_and_invalid_paths_are_rejected") {
        return;
    }
    let (_dir, paths) = fixture();
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    let path = agent_llm::AppConfig::default_path(&paths.config_dir);
    std::fs::write(&path, "[this is not valid TOML").unwrap();
    let bridge = HostBridge::spawn(paths.clone()).unwrap();
    let events = bridge.events();
    let initial = snapshot(&events, 0);
    assert!(initial.model_info["settings"]["config"]["error"].is_string());
    let request=bridge.send(Command::Settings(SettingsCommand::SaveProvider{original:None,id:"demo".into(),value:serde_json::json!({"kind":"openai_completions","base_url":"https://example.invalid","models":[]})})).unwrap();
    let _ = error_for(&events, request);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[this is not valid TOML"
    );
    let request = bridge
        .send(Command::Settings(SettingsCommand::SaveMcp {
            id: "../escape".into(),
            value: serde_json::json!({"command":"unused"}),
        }))
        .unwrap();
    assert!(error_for(&events, request).contains("名称"));
    assert!(!paths.config_dir.join("escape").exists());
}

#[test]
fn settings_mcp_transport_becomes_loadable_manifest_and_restores_credentials() {
    if isolated("settings_mcp_transport_becomes_loadable_manifest_and_restores_credentials") {
        return;
    }
    let (_dir, paths) = fixture();
    let bridge = HostBridge::spawn(paths.clone()).unwrap();
    let events = bridge.events();
    snapshot(&events, 0);
    let request=bridge.send(Command::Settings(SettingsCommand::SaveMcp{id:"demo-http".into(),value:serde_json::json!({"url":"https://example.invalid/mcp","headers":{"Authorization":"test-only-mcp-token"}})})).unwrap();
    let state = snapshot(&events, request);
    let row = state.model_info["settings"]["mcp"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "demo-http")
        .unwrap();
    assert_eq!(row["manifest"]["kind"], "mcp");
    assert_eq!(row["manifest"]["entry"]["headers"], "••••••••");
    assert!(!state.model_info.to_string().contains("test-only-mcp-token"));
    let mut edited = row["manifest"].clone();
    edited["entry"]["url"] = serde_json::json!("https://example.invalid/edited");
    let request = bridge
        .send(Command::Settings(SettingsCommand::SaveMcp {
            id: "demo-http".into(),
            value: edited,
        }))
        .unwrap();
    snapshot(&events, request);
    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(paths.config_dir.join("mcp/demo-http/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        stored["entry"]["headers"]["Authorization"],
        "test-only-mcp-token"
    );
    let request = bridge
        .send(Command::Settings(SettingsCommand::DeleteMcp(
            "demo-http".into(),
        )))
        .unwrap();
    snapshot(&events, request);
    assert!(
        !paths
            .config_dir
            .join("mcp/demo-http/manifest.json")
            .exists()
    );
}
