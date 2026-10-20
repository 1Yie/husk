use super::*;
use agent_ipc::{AgentState, UiEvent};
use agent_llm::types::StreamChunk;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

use agent_testkit::ScriptedProvider;

fn seed(dir: &std::path::Path, provider: Arc<ScriptedProvider>) -> SessionSeed {
    SessionSeed {
        workspace: dir.into(),
        provider,
        provider_name: "test".into(),
        model: "test-model".into(),
        permission: "auto".into(),
        agent_mode: "build".into(),
        thinking: Some("high".into()),
        config: AppConfig::default(),
    }
}

fn app_for(seed: &SessionSeed) -> App {
    let mut cfg = make_cfg(seed, None, 100).unwrap();
    cfg.memory_enabled = false;
    cfg.memory_distill = false;
    let (actor, channels) = SessionActor::spawn_with_id(cfg, 100);
    App::new(
        actor.command_sender(),
        channels.event_rx,
        actor.decision_writer(),
        actor.ask_channel(),
        actor.cancel_writer(),
        actor.queue_writer(),
        seed.provider_name.clone(),
        seed.model.clone(),
        "test".into(),
        false,
        seed.workspace.clone(),
        seed.permission.clone(),
        seed.agent_mode.clone(),
        seed.thinking.clone(),
        Vec::new(),
    )
}

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Vec<UiCommand> {
    app.on_term_event(Event::Key(KeyEvent::new(code, modifiers)))
}

fn meta(provider: Option<&str>, model: Option<&str>) -> SessionMeta {
    serde_json::from_value(serde_json::json!({
        "id": 7, "title": "test", "preview": "test", "updated_at": 1,
        "provider": provider, "model": model
    }))
    .unwrap()
}

#[tokio::test]
async fn new_session_displays_its_actual_mode_permissions_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0, id: Some("write".into()), name: Some("apply_patch".into()),
            args_delta: serde_json::json!({"patch": "*** Begin Patch\n*** Add File: new.txt\n+test\n*** End Patch\n"}).to_string(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]);
    provider.script_text("done");
    let seed = seed(dir.path(), provider);
    let mut app = app_for(&seed);
    app.agent_mode = "plan".into();
    app.permission_mode = "dontAsk".into();
    app.model = "other-model".into();
    app.thinking_level = Some("low".into());
    new_into(&mut app, &seed).await;
    // /new uses the boot defaults; every label must reset with the actor.
    assert_eq!(app.agent_mode, "build");
    assert_eq!(app.permission_mode, "auto");
    assert_eq!(app.model, "test-model");
    assert_eq!(app.thinking_level.as_deref(), Some("high"));
    app.dispatch(UiCommand::Prompt {
        text: "test".into(),
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(ev) = app.ev_rx.recv().await {
            match ev {
                UiEvent::StateChanged(AgentState::Finished) => return,
                UiEvent::StateChanged(AgentState::Failed(e)) => panic!("turn failed: {e}"),
                _ => {}
            }
        }
        panic!("actor ended before completing turn");
    })
    .await
    .unwrap();
    assert!(dir.path().join("new.txt").is_file());
}

#[test]
fn resume_resolves_provider_and_model_options_together() {
    let dir = tempfile::tempdir().unwrap();
    let mut seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    seed.config.providers.insert("other".into(), serde_json::from_value(serde_json::json!({
            "kind": "openai_completions", "base_url": "http://127.0.0.1:1/v1", "api_key": "test",
        "models": [{"id": "other-model", "contextWindow": 8192, "input": ["text", "image"], "maxTokens": 321}]
    })).unwrap());
    let cfg = make_cfg(&seed, Some(&meta(Some("other"), Some("other-model"))), 1).unwrap();
    assert_eq!(cfg.provider_name, "other");
    assert_eq!(cfg.model, "other-model");
    assert!(!Arc::ptr_eq(&cfg.provider, &seed.provider));
    assert_eq!(cfg.context_window, Some(8192));
    assert!(cfg.model_input.contains(&"image".into()));
    assert_eq!(cfg.model_params.unwrap().max_tokens, Some(321));
    assert!(make_cfg(&seed, Some(&meta(Some("missing"), Some("other-model"))), 1).is_err());
}

#[test]
fn thinking_defaults_and_resume_default_reach_the_engine_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    assert_eq!(
        make_cfg(&seed, None, 1).unwrap().thinking_level.as_deref(),
        Some("high")
    );
    let mut saved = meta(None, None);
    // A saved default (None) must clear another session's custom level.
    let cfg = make_cfg(&seed, Some(&saved), 1).unwrap();
    assert_eq!(cfg.thinking_level, None);
    let mut app = app_for(&seed);
    apply_cfg_labels(&mut app, &cfg);
    assert_eq!(app.thinking_level, None);
    saved.thinking_level = Some("low".into());
    assert_eq!(
        make_cfg(&seed, Some(&saved), 1)
            .unwrap()
            .thinking_level
            .as_deref(),
        Some("low")
    );
}

#[test]
fn ctrl_u_clears_nonempty_buffer_from_start_and_middle() {
    let dir = tempfile::tempdir().unwrap();
    let seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    let mut app = app_for(&seed);
    for move_key in [KeyCode::Home, KeyCode::Left] {
        app.composer.set_text("abc\n中文 xyz");
        key(&mut app, move_key, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(app.composer.text(), "");
        app.composer.insert_char('x');
        assert_eq!(app.composer.text(), "x");
    }
}

#[test]
fn model_picker_preserves_navigation_and_restores_providers() {
    let dir = tempfile::tempdir().unwrap();
    let seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    let mut app = app_for(&seed);
    app.model_catalog = vec![
        ("test".into(), vec!["test-model".into(), "alternate".into()]),
        ("other".into(), vec!["other-model".into()]),
    ];
    app.slash("/model").unwrap();
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert!(matches!(
        &app.modal,
        Some(modals::Modal::Models {
            stage: 1,
            selected: 1,
            ..
        })
    ));
    let commands = key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        matches!(&commands[..], [UiCommand::SetModel { provider, model }] if provider == "test" && model == "alternate")
    );
    for back in [KeyCode::Esc, KeyCode::Left] {
        app.slash("/model").unwrap();
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut app, back, KeyModifiers::NONE);
        assert!(
            matches!(&app.modal, Some(modals::Modal::Models { stage: 0, items, .. }) if items == &vec!["test".to_string(), "other".to_string()])
        );
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let commands = key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(
            matches!(&commands[..], [UiCommand::SetModel { provider, model }] if provider == "other" && model == "other-model")
        );
    }
}

#[test]
fn composer_keeps_wrapped_and_multiline_cursor_visible() {
    use ratatui::backend::{Backend, TestBackend};
    let dir = tempfile::tempdir().unwrap();
    let seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    let mut app = app_for(&seed);
    let mut terminal = Terminal::new(TestBackend::new(40, 16)).unwrap();
    for text in [
        format!("{}END", "中".repeat(80)),
        "a\nb\nc\nd\ne\nf\nEND".into(),
    ] {
        app.composer.set_text(&text);
        terminal
            .draw(|frame| render::draw(frame, &mut app))
            .unwrap();
        let cursor = terminal.backend_mut().get_cursor_position().unwrap();
        assert!(
            cursor.x < 39 && cursor.y < 15,
            "cursor must stay in the input pane: {cursor:?}"
        );
        let buffer_text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(buffer_text.contains("END"), "edited tail must be visible");
    }
    for width in [1, 2, 3, 4, 8, 40] {
        let mut terminal = Terminal::new(TestBackend::new(width, 16)).unwrap();
        terminal
            .draw(|frame| render::draw(frame, &mut app))
            .unwrap();
        let cursor = terminal.backend_mut().get_cursor_position().unwrap();
        assert!(cursor.x < width);
    }
}

#[test]
fn markdown_preserves_heading_block_boundaries() {
    let text: Vec<String> = markdown::render("# 标题\n\n正文\n\n## first\n\n## second")
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(text, ["标题", "正文", "first", "second"]);
}

#[test]
fn streamed_activity_keeps_visible_tail_and_reflows_after_resize() {
    let dir = tempfile::tempdir().unwrap();
    let seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    let mut app = app_for(&seed);
    let text = "abcdefgh中文\n".repeat(200);
    for chunk in text.chars().collect::<Vec<_>>().chunks(7) {
        app.on_ui_event(UiEvent::TextDelta {
            text: chunk.iter().collect(),
            parent: None,
        });
    }
    for width in [12, 30, 8, 12] {
        app.set_width(width);
        let expected: Vec<ratatui::text::Line> = transcript::wrap_lines(
            text.lines()
                .map(|l| ratatui::text::Line::from(l.to_string()))
                .collect(),
            width,
        );
        let visible = app.activity_lines(width as usize, 4);
        assert_eq!(visible, expected[expected.len() - 4..]);
        assert_eq!(app.activity_lines(width as usize, 4), visible);
    }
    app.on_ui_event(UiEvent::ReasoningDelta {
        text: "reason".into(),
        parent: None,
    });
    app.on_ui_event(UiEvent::TextDelta {
        text: "child".into(),
        parent: Some("delegate".into()),
    });
    let visible = app.activity_lines(30, 2);
    let plain: Vec<String> = visible
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    assert_eq!(plain, ["  reason", "  ↳ child"]);
}

fn plain_lines(lines: &[ratatui::text::Line]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

fn tool_start(name: &str, preview: &str) -> UiEvent {
    UiEvent::ToolCallStarted {
        name: name.into(),
        args_preview: preview.into(),
        parent: None,
        child_id: None,
    }
}

fn tool_done(name: &str, content: &str) -> UiEvent {
    UiEvent::ToolCallFinished {
        name: name.into(),
        ok: true,
        content: content.into(),
        ui_type: None,
        parent: None,
        child_id: None,
    }
}

#[test]
fn finished_tool_appears_once_and_leaves_the_live_region() {
    let (_dir, mut app) = app_and_dir();
    app.set_width(80);
    app.on_ui_event(tool_start("list_dir", "crates"));
    app.on_ui_event(tool_done("list_dir", "a.rs\nb.rs"));
    let committed = plain_lines(&app.pending);
    assert_eq!(
        committed.iter().filter(|l| l.contains("list_dir")).count(),
        1
    );
    assert!(app.live.is_empty(), "finished tool still in live region");
}

#[test]
fn batch_children_are_grouped_under_their_parent_with_previews() {
    let (_dir, mut app) = app_and_dir();
    app.set_width(80);
    app.on_ui_event(tool_start("batch_execute", ""));
    for file in ["a.rs", "b.rs"] {
        app.on_ui_event(UiEvent::ToolCallStarted {
            name: "smart_read".into(),
            args_preview: file.into(),
            parent: Some("batch_execute".into()),
            child_id: None,
        });
        app.on_ui_event(UiEvent::ToolCallFinished {
            name: "smart_read".into(),
            ok: true,
            content: "x".into(),
            ui_type: None,
            parent: Some("batch_execute".into()),
            child_id: None,
        });
    }
    assert!(
        plain_lines(&app.pending).is_empty(),
        "children committed before parent finished"
    );
    app.on_ui_event(tool_done("batch_execute", ""));
    let committed = plain_lines(&app.pending);
    let parent = committed
        .iter()
        .position(|l| l.contains("batch_execute"))
        .unwrap();
    assert!(committed[parent + 1].contains("smart_read") && committed[parent + 1].contains("a.rs"));
    assert!(committed[parent + 2].contains("smart_read") && committed[parent + 2].contains("b.rs"));
}

#[test]
fn reasoning_is_committed_before_the_tool_it_precedes() {
    let (_dir, mut app) = app_and_dir();
    app.set_width(80);
    app.on_ui_event(UiEvent::ReasoningDelta {
        text: "first look at the dir".into(),
        parent: None,
    });
    app.on_ui_event(tool_start("list_dir", "crates"));
    app.on_ui_event(tool_done("list_dir", ""));
    let committed = plain_lines(&app.pending);
    let reason = committed
        .iter()
        .position(|l| l.contains("first look"))
        .expect("reasoning not committed");
    let tool = committed
        .iter()
        .position(|l| l.contains("list_dir"))
        .unwrap();
    assert!(reason < tool, "reasoning committed after the tool");
}

fn app_and_dir() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let seed = seed(dir.path(), Arc::new(ScriptedProvider::new()));
    let app = app_for(&seed);
    (dir, app)
}

#[test]
fn cache_hit_share_is_a_clamped_percentage() {
    use crate::events::cache_hit_pct;
    assert_eq!(cache_hit_pct(0, 0), None);
    assert_eq!(cache_hit_pct(1000, 750), Some(75));
    assert_eq!(cache_hit_pct(100, 400), Some(100));
}
