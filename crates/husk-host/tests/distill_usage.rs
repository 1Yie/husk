//! The post-turn distillation pass is a side channel: it records its own token
//! spend, and it follows the session's model.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_llm::types::StreamChunk;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

fn config(
    dir: &std::path::Path,
    provider: Arc<agent_testkit::ScriptedProvider>,
    model: &str,
) -> SessionConfig {
    SessionConfig {
        workspace_root: dir.to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider,
        model: model.into(),
        temperature: 0.0,
        permission_mode: "bypassPermissions".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: true,
        memory_distill: true,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    }
}

/// The distill reply: strict JSON plus the usage a real wire would report.
fn distill_script(prompt_tokens: u32, completion_tokens: u32) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ContentDelta(
            r#"{"task":"wire the distiller","facts":[],"persona":{}}"#.into(),
        ),
        StreamChunk::Done {
            prompt_tokens: Some(prompt_tokens),
            completion_tokens: Some(completion_tokens),
            cached_tokens: None,
        },
    ]
}

#[test]
fn the_distill_pass_records_its_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(agent_testkit::ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ContentDelta("done".into()),
        StreamChunk::Done {
            prompt_tokens: Some(10),
            completion_tokens: Some(2),
            cached_tokens: None,
        },
    ]);
    provider.push_script(distill_script(120, 40));

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, _channels) =
        rt.block_on(async { SessionActor::spawn(config(dir.path(), provider.clone(), "m1")) });
    rt.block_on(actor.handle(UiCommand::Prompt {
        text: "do the thing".into(),
    }));

    // The pass is fire-and-forget — wait for it to land in the store.
    let store = agent_store::SessionStore::open(dir.path()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let usage = loop {
        if let Some(u) = store.meta(0).and_then(|m| m.distill) {
            break u;
        }
        assert!(Instant::now() < deadline, "the distill pass never recorded");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(usage.calls, 1);
    assert_eq!(usage.prompt, 120);
    assert_eq!(usage.completion, 40);

    // A second turn: its boundary write must not erase the recorded spend,
    // and the next pass accumulates on top of it.
    provider.push_script(vec![
        StreamChunk::ContentDelta("again".into()),
        StreamChunk::Done {
            prompt_tokens: Some(12),
            completion_tokens: Some(3),
            cached_tokens: None,
        },
    ]);
    provider.push_script(distill_script(100, 30));
    rt.block_on(actor.handle(UiCommand::Prompt {
        text: "and again".into(),
    }));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let d = store
            .meta(0)
            .and_then(|m| m.distill)
            .expect("still recorded");
        if d.calls >= 2 {
            assert_eq!((d.prompt, d.completion), (220, 70), "{d:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the second distill never landed: {d:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn distillation_follows_the_session_model() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(agent_testkit::ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ContentDelta("done".into()),
        StreamChunk::Done {
            prompt_tokens: Some(10),
            completion_tokens: Some(2),
            cached_tokens: None,
        },
    ]);
    provider.push_script(distill_script(80, 20));
    // The second turn, then its distill pass.
    provider.push_script(vec![
        StreamChunk::ContentDelta("again".into()),
        StreamChunk::Done {
            prompt_tokens: Some(12),
            completion_tokens: Some(3),
            cached_tokens: None,
        },
    ]);
    provider.push_script(distill_script(90, 25));

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, _channels) =
        rt.block_on(async { SessionActor::spawn(config(dir.path(), provider.clone(), "m1")) });
    rt.block_on(actor.handle(UiCommand::Prompt {
        text: "first".into(),
    }));
    let store = agent_store::SessionStore::open(dir.path()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.meta(0).and_then(|m| m.distill).is_none() {
        assert!(Instant::now() < deadline, "first distill never landed");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Switch models mid-session; the distill pass must follow.
    rt.block_on(actor.handle(UiCommand::SetModel {
        provider: "test".into(),
        model: "m2".into(),
    }));
    rt.block_on(actor.handle(UiCommand::Prompt {
        text: "second".into(),
    }));

    let deadline = Instant::now() + Duration::from_secs(10);
    while store
        .meta(0)
        .and_then(|m| m.distill)
        .map(|d| d.calls)
        .unwrap_or(0)
        < 2
    {
        assert!(Instant::now() < deadline, "the second distill never landed");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Every request, in order: turn(m1), distill(m1), turn(m2), distill(m2).
    let models: Vec<String> = provider
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|(m, _, _)| m.clone())
        .collect();
    assert_eq!(
        models,
        vec!["m1", "m1", "m2", "m2"],
        "the distill call must use the model the session now runs"
    );
}
