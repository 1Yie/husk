use std::{collections::HashMap, sync::Arc};

use husk_host::{
    command::UiCommand,
    session::{SessionActor, SessionConfig},
};

#[tokio::test]
async fn model_switch_and_settings_persist_only_supported_thinking_levels() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
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
active_model = "b"
[providers.test]
type = "openai_compat"
base_url = "https://example.invalid/v1"
api_key = "test"
[[providers.test.models]]
id = "a"
[providers.test.models.thinking_level_map]
low = "low"
high = "high"
max = "max"
[[providers.test.models]]
id = "b"
[providers.test.models.thinking_level_map]
medium = "medium"
xhigh = "xhigh"
[[providers.test.models]]
id = "plain"
"#,
    )
    .unwrap();
    let (mut actor, _channels) = SessionActor::spawn(SessionConfig {
        workspace_root: root.clone(),
        paths,
        provider: Arc::new(agent_testkit::ScriptedProvider::new()),
        model: "b".into(),
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
    });
    let level = actor.thinking_writer();
    assert_eq!(level.read().unwrap().as_deref(), Some("xhigh"));
    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "a".into(),
        })
        .await;
    assert_eq!(level.read().unwrap().as_deref(), Some("low"));
    actor
        .handle(UiCommand::SetThinkingLevel {
            level: "xhigh".into(),
        })
        .await;
    assert_eq!(level.read().unwrap().as_deref(), Some("low"));
    assert_eq!(
        actor.history().last().unwrap().content.as_deref(),
        Some("已设置思考推理强度: low")
    );

    let store = agent_store::SessionStore::open(&root).unwrap();
    let persisted = store
        .list()
        .into_iter()
        .find(|meta| meta.id == actor.session_id())
        .unwrap();
    assert_eq!(persisted.model.as_deref(), Some("a"));
    assert_eq!(persisted.thinking_level.as_deref(), Some("low"));

    // Removing the current tier in config must revalidate it on reload too.
    let config_path = temp.path().join("config/config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(&config_path, config.replace("low = \"low\"", "low = \"\"")).unwrap();
    let history_len = actor.history_len();
    actor.handle(UiCommand::ReloadModel).await;
    assert_eq!(actor.history_len(), history_len);
    assert_eq!(level.read().unwrap().as_deref(), Some("high"));
    let reloaded = store
        .list()
        .into_iter()
        .find(|meta| meta.id == actor.session_id())
        .unwrap();
    assert_eq!(reloaded.thinking_level.as_deref(), Some("high"));

    actor
        .handle(UiCommand::SetThinkingLevel {
            level: "high".into(),
        })
        .await;
    assert_eq!(level.read().unwrap().as_deref(), Some("high"));
    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "b".into(),
        })
        .await;
    assert_eq!(level.read().unwrap().as_deref(), Some("medium"));
    actor
        .handle(UiCommand::SetThinkingLevel {
            level: "off".into(),
        })
        .await;
    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "a".into(),
        })
        .await;
    assert_eq!(level.read().unwrap().as_deref(), Some("off"));

    actor
        .handle(UiCommand::SetModel {
            provider: "test".into(),
            model: "plain".into(),
        })
        .await;
    actor
        .handle(UiCommand::SetThinkingLevel {
            level: "xhigh".into(),
        })
        .await;
    assert_eq!(*level.read().unwrap(), None);
    let persisted = store
        .list()
        .into_iter()
        .find(|meta| meta.id == actor.session_id())
        .unwrap();
    assert_eq!(persisted.thinking_level, None);
    store.delete_workspace().unwrap();
}
