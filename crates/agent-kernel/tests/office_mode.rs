//! Office-mode acceptance: the schema the model is offered carries the
//! `office_*` document tools ON TOP of the full programming set, the mode
//! round-trips through `from_str`/`as_str`, and the prompt block names the
//! office contract. Mirrors `plan_mode.rs` — spawn a real SessionActor in
//! office mode and read the tool names the provider stub saw.

use std::sync::Arc;

use agent_ipc::UiCommand;
use agent_kernel::mode::AgentMode;
use agent_kernel::session::{SessionActor, SessionConfig};
mod common;
use common::ScriptedProvider;

fn spawn(
    dir: &std::path::Path,
    mode: &str,
    stub: Arc<ScriptedProvider>,
) -> (SessionActor, agent_kernel::channels::UiChannels) {
    SessionActor::spawn(SessionConfig {
        workspace_root: dir.to_path_buf(),
        provider: stub,
        model: "test-model".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: mode.into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    })
}

/// Office mode offers the `office_*` tools AND keeps the full programming set
/// (the mode is a document-production contract on top of build, not a cut-down
/// toolbox).
#[tokio::test]
async fn office_schema_offers_document_tools_plus_full_set() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("做一个表格。");
    let (mut actor, _ch) = spawn(dir.path(), "office", stub.clone());

    actor
        .handle(UiCommand::Prompt {
            text: "新建一个 xlsx".into(),
        })
        .await;

    let names = stub
        .tool_names
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("a request went out");
    // The office document tools.
    for t in [
        "office_create",
        "office_get",
        "office_query",
        "office_set",
        "office_add",
        "office_remove",
        "office_move",
        "office_batch",
        "office_merge",
        "office_import",
        "office_view",
        "office_open",
        "office_save",
        "office_close",
        "office_validate",
        "office_help",
        "office_skill",
        "office_raw",
        "office_exec",
        "image_search",
        "web_download",
        "view_image",
    ] {
        assert!(names.contains(&t.to_string()), "missing {t}: {names:?}");
    }
    // The full programming set is still there (office = build + docs).
    for t in [
        "smart_read",
        "apply_patch",
        "fuzzy_patch",
        "bash",
        "smart_grep",
    ] {
        assert!(names.contains(&t.to_string()), "missing {t}: {names:?}");
    }
    // Office is not goal/plan — no contract tools.
    for absent in ["submit_plan", "goal_complete", "goal_blocked"] {
        assert!(
            !names.contains(&absent.to_string()),
            "{absent} leaked into office mode: {names:?}"
        );
    }
}

/// Build mode must NOT offer the office tools — they stay scoped to the
/// office registry so the programming modes aren't polluted.
#[tokio::test]
async fn build_schema_has_no_office_tools() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Arc::new(ScriptedProvider::new());
    stub.script_text("写代码。");
    let (mut actor, _ch) = spawn(dir.path(), "build", stub.clone());

    actor.handle(UiCommand::Prompt { text: "hi".into() }).await;

    let names = stub
        .tool_names
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("a request went out");
    for t in [
        "office_create",
        "office_batch",
        "office_merge",
        "image_search",
        "web_download",
        "view_image",
    ] {
        assert!(
            !names.contains(&t.to_string()),
            "{t} leaked into build mode: {names:?}"
        );
    }
}

/// The mode string/label/contract round-trips consistently.
#[test]
fn office_mode_str_roundtrip() {
    let m = AgentMode::from_str("office");
    assert_eq!(m, AgentMode::Office);
    assert_eq!(m.as_str(), "office");
    assert!(m.prompt_block().contains("office"));
    assert!(m.prompt_block().contains(".docx"));
    assert!(m.prompt_block().contains(".xlsx"));
    assert!(m.prompt_block().contains(".pptx"));
}
