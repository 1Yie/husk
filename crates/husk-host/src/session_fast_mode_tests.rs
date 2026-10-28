use super::*;
use std::collections::HashMap;

fn fixture() -> (tempfile::TempDir, agent_paths::AppPaths, SessionActor) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let paths = agent_paths::AppPaths::new(
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
]
"#,
    )
    .unwrap();
    let store = agent_store::SessionStore::open(&root).unwrap();
    store
        .upsert_meta(agent_store::SessionMeta {
            id: 71,
            ..Default::default()
        })
        .unwrap();
    let cfg = SessionConfig {
        workspace_root: root.clone(),
        paths: paths.clone(),
        provider: Arc::new(agent_testkit::ScriptedProvider::new()),
        model: "a".into(),
        provider_name: "test".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
        thinking_level: Some("xhigh".into()),
        thinking_level_map: Some(HashMap::from([
            ("medium".into(), Some("medium".into())),
            ("xhigh".into(), Some("xhigh".into())),
        ])),
        track_dirty: false,
        context_window: None,
        compact_at: None,
        approval_timeout: None,
        question_timeout: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
    };
    let (actor, _channels) = SessionActor::spawn_with_id(cfg, 71);
    (temp, paths, actor)
}

#[tokio::test]
async fn legacy_restore_defaults_off_and_supported_model_switch_preserves_preference() {
    let (_temp, _paths, mut actor) = fixture();
    assert!(!*actor.fast_mode_writer().read().unwrap());
    assert!(!actor.applied_fast_mode);
    // Manager's durable-accept seam: the command only wakes the pump.
    write_fast_mode(actor.store.as_ref().unwrap(), 71, true).unwrap();
    *actor.fast_mode_writer().write().unwrap() = true;
    actor.handle(UiCommand::SetFastMode { enabled: true }).await;
    assert!(actor.applied_fast_mode);
    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "b".into(),
        })
        .await;
    assert!(actor.applied_fast_mode);
    assert!(read_fast_mode(actor.store.as_ref().unwrap(), 71));
    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "plain".into(),
        })
        .await;
    assert!(!actor.applied_fast_mode);
    assert!(!*actor.fast_mode_writer().read().unwrap());
    assert!(!read_fast_mode(actor.store.as_ref().unwrap(), 71));
    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "a".into(),
        })
        .await;
    assert!(
        !actor.applied_fast_mode,
        "an unsupported hop must not silently re-enable Fast"
    );
}

#[tokio::test]
async fn reload_removing_capability_clears_live_and_persisted_fast_mode() {
    let (_temp, paths, mut actor) = fixture();
    *actor.fast_mode_writer().write().unwrap() = true;
    write_fast_mode(actor.store.as_ref().unwrap(), 71, true).unwrap();
    actor.handle(UiCommand::SetFastMode { enabled: true }).await;
    let config = paths.config_dir.join("config.toml");
    let without_fast = std::fs::read_to_string(&config)
        .unwrap()
        .replace(r#", fastMode = { mode = "priority" }"#, "");
    std::fs::write(config, without_fast).unwrap();
    actor.handle(UiCommand::ReloadModel).await;
    assert!(!actor.applied_fast_mode);
    assert!(!*actor.fast_mode_writer().read().unwrap());
    assert!(!read_fast_mode(actor.store.as_ref().unwrap(), 71));
}
