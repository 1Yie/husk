//! `delegate` — parallel read-only fan-out, end to end: real child engines,
//! scripted provider, no network.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

/// The steer registry is process-wide, so every test that runs a child takes
/// this lock. Without it a steer test can pick up, or be blocked by, a child
/// another test started, and the outcome depends on scheduling.
static DELEGATE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

use agent_kernel::channels::UiSink;
use agent_kernel::tools::delegate::{ChildRoute, SubagentSpawner};
use agent_kernel::tools::registry::ToolClass;
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use common::ScriptedProvider;

fn spawner(provider: Arc<dyn agent_llm::LlmProvider>) -> SubagentSpawner {
    let full = ToolRegistry::with_builtins()
        .filtered(|s| s.name != "delegate" && s.class != ToolClass::HumanInteraction);
    let readonly = full.readonly_only();
    let route = Arc::new(std::sync::RwLock::new(ChildRoute {
        provider,
        model: "test-model".into(),
        temperature: 0.0,
        context_window: 256_000,
        thinking_level: None,
        thinking_level_map: None,
        model_params: agent_llm::ModelParams::default(),
        model_input: Vec::new(),
        models: std::collections::HashMap::new(),
    }));
    SubagentSpawner::new(route, Arc::new(full), Arc::new(readonly))
}

fn ctx_with(dir: &std::path::Path, spawner: SubagentSpawner) -> Arc<ToolCtx> {
    let mut ctx = ToolCtx::new(dir);
    ctx.install_paths(agent_paths::AppPaths::new(dir, dir, ".husk"));
    ctx.install_subagent(spawner);
    Arc::new(ctx)
}

#[tokio::test]
async fn parallel_tasks_run_together_and_merge_in_input_order() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("alpha report");
    provider.script_text("beta report");
    let ctx = ctx_with(dir.path(), spawner(provider.clone()));

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    let out = (spec.exec)(
        serde_json::json!({"tasks": ["inspect alpha", "inspect beta"], "readonly": true}),
        ctx,
    )
    .await
    .expect("parallel delegation must succeed");

    assert!(out.content.contains("2 ok, 0 failed"), "{}", out.content);
    assert!(
        out.content.contains("alpha report") && out.content.contains("beta report"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("── [0] ok") && out.content.contains("── [1] ok"),
        "{}",
        out.content
    );
    // One provider call per child — the fan-out really ran both.
    assert_eq!(provider.calls.lock().unwrap().len(), 2);
}

/// The approval story the tool promises: parallel children are read-only, so a
/// single approval can never cover several writers.
#[tokio::test]
async fn write_capable_parallel_tasks_are_refused_before_anything_spawns() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    let ctx = ctx_with(dir.path(), spawner(provider.clone()));

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    let err = (spec.exec)(
        serde_json::json!({"tasks": ["write one", "write two"]}),
        ctx,
    )
    .await
    .expect_err("non-readonly tasks must be refused");

    assert!(format!("{err}").contains("readonly: true"), "{err}");
    assert_eq!(provider.calls.lock().unwrap().len(), 0, "nothing may spawn");
}

/// Each child gets its own card in the parent's stream (nested under `delegate`),
/// so a fan-out is visible instead of being one silent wait.
#[tokio::test]
async fn every_child_emits_a_nested_card() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("one");
    provider.script_text("two");
    let (sink, mut rx) = UiSink::channel();
    let mut ctx = ToolCtx::new(dir.path());
    ctx.install_paths(agent_paths::AppPaths::new(dir.path(), dir.path(), ".husk"));
    ctx.install_subagent(spawner(provider));
    ctx.install_ui(sink);

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    let _ = (spec.exec)(
        serde_json::json!({"tasks": ["alpha", "beta"], "readonly": true}),
        Arc::new(ctx),
    )
    .await
    .expect("parallel delegation");

    let mut started = Vec::new();
    let mut finished = Vec::new();
    let mut text: Vec<(Option<String>, String)> = Vec::new();
    let mut reasoning: Vec<Option<String>> = Vec::new();
    while let Some(ev) = rx.try_recv() {
        match ev {
            agent_ipc::events::UiEvent::ToolCallStarted { name, parent, .. } => {
                started.push((name, parent))
            }
            agent_ipc::events::UiEvent::ToolCallFinished {
                name, ok, parent, ..
            } => finished.push((name, ok, parent)),
            agent_ipc::events::UiEvent::TextDelta { text: t, parent } => text.push((parent, t)),
            agent_ipc::events::UiEvent::ReasoningDelta { parent, .. } => reasoning.push(parent),
            _ => {}
        }
    }
    assert_eq!(started.len(), 2, "{started:?}");
    assert_eq!(finished.len(), 2, "{finished:?}");
    for (name, parent) in &started {
        assert_eq!(parent.as_deref(), Some("delegate"), "{name}");
    }
    assert!(
        started.iter().any(|(n, _)| n == "subagent #1"),
        "{started:?}"
    );
    assert!(
        started.iter().any(|(n, _)| n == "subagent #2"),
        "{started:?}"
    );
    assert!(finished.iter().all(|(_, ok, _)| *ok), "{finished:?}");

    // The children's prose arrives tagged with their own labels, so the UI can
    // stream it under the right capsule instead of into the turn's draft.
    let mut per_child: std::collections::BTreeMap<String, String> = Default::default();
    for (parent, t) in &text {
        let parent = parent.clone().expect("child text must carry its label");
        per_child.entry(parent).or_default().push_str(t);
    }
    assert_eq!(per_child.len(), 2, "{text:?}");
    let mut bodies: Vec<&str> = per_child.values().map(String::as_str).collect();
    bodies.sort_unstable();
    assert_eq!(bodies, vec!["one", "two"], "{text:?}");
    assert!(text.iter().all(|(p, _)| p.is_some()), "{text:?}");
    assert!(
        reasoning.iter().all(|p| p.as_deref().is_none_or(|_| true)),
        "{reasoning:?}"
    );
}

/// A spawner whose route the test can rewrite mid-flight, the way a session's
/// `SetModel` does. Returns the shared route so the test can hot-swap it.
fn spawner_with_route(
    provider: Arc<dyn agent_llm::LlmProvider>,
) -> (SubagentSpawner, Arc<std::sync::RwLock<ChildRoute>>) {
    let full = ToolRegistry::with_builtins()
        .filtered(|s| s.name != "delegate" && s.class != ToolClass::HumanInteraction);
    let readonly = full.readonly_only();
    let route = Arc::new(std::sync::RwLock::new(ChildRoute {
        provider,
        model: "model-a".into(),
        temperature: 0.0,
        context_window: 256_000,
        thinking_level: Some("high".into()),
        thinking_level_map: None,
        model_params: agent_llm::ModelParams::default(),
        model_input: Vec::new(),
        models: std::collections::HashMap::new(),
    }));
    let spawner = SubagentSpawner::new(route.clone(), Arc::new(full), Arc::new(readonly));
    (spawner, route)
}

/// Writes a `.husk/agents/<name>.md` manifest under the workspace root.
fn manifest(dir: &std::path::Path, name: &str, frontmatter: &str) {
    let agents = dir.join(".husk/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join(format!("{name}.md")),
        format!("---\nname: {name}\ndescription: test\n{frontmatter}---\n\nBODY\n"),
    )
    .unwrap();
}

/// A `tools:` allowlist must narrow the child's real request, not just parse.
/// The child's `tools` schema is the only thing the model can call, so that is
/// what has to shrink.
#[tokio::test]
async fn manifest_tools_allowlist_narrows_the_child_request() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    manifest(dir.path(), "scout", "tools: [smart_read, smart_grep]\n");
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("found it");
    let (spawner, _route) = spawner_with_route(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(
        serde_json::json!({"task": "look around", "agent": "scout"}),
        ctx,
    )
    .await
    .expect("allowlisted child must run");

    let names = provider.tool_names.lock().unwrap();
    let mut seen = names[0].clone();
    seen.sort();
    assert_eq!(
        seen,
        vec!["smart_grep".to_string(), "smart_read".to_string()],
        "{seen:?}"
    );
}

/// A `tools:` entry that names no real tool is refused before anything spawns,
/// so the author learns about the typo instead of getting a silently weaker child.
#[tokio::test]
async fn unknown_tool_in_manifest_is_refused_before_spawning() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    manifest(dir.path(), "typo", "tools: [raed]\n");
    let provider = Arc::new(ScriptedProvider::new());
    let (spawner, _route) = spawner_with_route(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    let err = (spec.exec)(
        serde_json::json!({"task": "go", "agent": "typo"}),
        ctx,
    )
    .await
    .expect_err("unknown tool name must be refused");

    assert!(format!("{err}").contains("raed"), "{err}");
    assert_eq!(provider.calls.lock().unwrap().len(), 0, "nothing may spawn");
}

/// The builtin `review` is read-only by registry: even with no `readonly: true`
/// on the call, its child must not be offered any write tool.
#[tokio::test]
async fn builtin_review_child_is_offered_no_write_tools() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("no issues");
    let (spawner, _route) = spawner_with_route(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(
        serde_json::json!({"task": "review it", "agent": "review"}),
        ctx,
    )
    .await
    .expect("review must run");

    let names = provider.tool_names.lock().unwrap();
    for write in ["fuzzy_patch", "apply_patch", "bash"] {
        assert!(
            !names[0].iter().any(|n| n == write),
            "review offered `{write}`: {:?}",
            names[0]
        );
    }
    assert!(names[0].iter().any(|n| n == "smart_read"), "{:?}", names[0]);
}

/// Hot-swap reaches children: a child started after the route changes runs on
/// the new model and carries the new reasoning level. A child already in flight
/// keeps the snapshot it started with (covered by the same route read).
#[tokio::test]
async fn child_follows_hot_swapped_model_and_thinking_level() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("first");
    provider.script_text("second");
    let (spawner, route) = spawner_with_route(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);
    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();

    (spec.exec)(serde_json::json!({"task": "one"}), ctx.clone())
        .await
        .expect("first child");

    {
        let mut r = route.write().unwrap();
        r.model = "model-b".into();
        r.thinking_level = Some("low".into());
    }
    (spec.exec)(serde_json::json!({"task": "two"}), ctx)
        .await
        .expect("second child");

    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls[0].0, "model-a");
    assert_eq!(calls[1].0, "model-b");
    let efforts = provider.reasoning_efforts.lock().unwrap();
    assert_eq!(efforts[0].as_deref(), Some("high"));
    assert_eq!(efforts[1].as_deref(), Some("low"));
}

/// A spawner whose provider declares `model-a` (the parent's) and `model-b`
/// (a cheaper model with a smaller window), so manifests can name either.
fn spawner_with_models(
    provider: Arc<dyn agent_llm::LlmProvider>,
) -> (SubagentSpawner, Arc<std::sync::RwLock<ChildRoute>>) {
    let (spawner, route) = spawner_with_route(provider);
    route.write().unwrap().models.insert(
        "model-b".into(),
        agent_kernel::tools::delegate::ChildModel {
            context_window: 8_000,
            thinking_level_map: None,
            model_params: agent_llm::ModelParams::default(),
            model_input: vec!["text".into()],
        },
    );
    (spawner, route)
}

/// A manifest `model:` must reach the child's request, and the child must carry
/// that model's own window rather than the parent's.
#[tokio::test]
async fn manifest_model_override_reaches_the_child_request() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    manifest(dir.path(), "cheap", "model: model-b\n");
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("ok");
    let (spawner, _route) = spawner_with_models(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(serde_json::json!({"task": "scan", "agent": "cheap"}), ctx)
        .await
        .expect("declared model must run");

    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls[0].0, "model-b");
}

/// A `model:` the provider does not declare is refused before spawning, so a
/// typo cannot quietly run the child on the parent's model.
#[tokio::test]
async fn undeclared_manifest_model_is_refused_before_spawning() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    manifest(dir.path(), "ghost", "model: model-zzz\n");
    let provider = Arc::new(ScriptedProvider::new());
    let (spawner, _route) = spawner_with_models(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    let err = (spec.exec)(serde_json::json!({"task": "go", "agent": "ghost"}), ctx)
        .await
        .expect_err("undeclared model must be refused");

    assert!(format!("{err}").contains("model-zzz"), "{err}");
    assert_eq!(provider.calls.lock().unwrap().len(), 0, "nothing may spawn");
}

/// `thinking:` must be a known level; an unknown one is refused, and a known one
/// replaces the parent's level for that child only.
#[tokio::test]
async fn manifest_thinking_is_validated_and_applied() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    manifest(dir.path(), "shallow", "thinking: low\n");
    manifest(dir.path(), "garbled", "thinking: ultra\n");
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("ok");
    let (spawner, _route) = spawner_with_models(provider.clone());
    let ctx = ctx_with(dir.path(), spawner);
    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();

    let err = (spec.exec)(serde_json::json!({"task": "x", "agent": "garbled"}), ctx.clone())
        .await
        .expect_err("unknown thinking level must be refused");
    assert!(format!("{err}").contains("ultra"), "{err}");
    assert_eq!(provider.calls.lock().unwrap().len(), 0, "nothing may spawn");

    (spec.exec)(serde_json::json!({"task": "x", "agent": "shallow"}), ctx)
        .await
        .expect("known level must run");
    let efforts = provider.reasoning_efforts.lock().unwrap();
    assert_eq!(efforts[0].as_deref(), Some("low"));
}

/// A steer delivered to a running child reaches the model on its next request,
/// as the user's interruption. The provider holds the child on its first request,
/// so the child is genuinely mid-turn while the steer is sent, and steering is
/// drained before the next request.
#[tokio::test]
async fn steer_reaches_a_running_child_on_its_next_request() {
    use agent_llm::types::StreamChunk;
    let _serial = DELEGATE_SERIAL.lock().await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("call_1".into()),
            name: Some("smart_read".into()),
            args_delta: r#"{"path":"notes.txt"}"#.into(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]);
    provider.script_text("done");
    let gate = Arc::new(tokio::sync::Notify::new());
    *provider.gate.lock().unwrap() = Some(gate.clone());
    let ctx = ctx_with(dir.path(), spawner(provider.clone()));

    let run = tokio::spawn(async move {
        let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
        (spec.exec)(serde_json::json!({"task": "read notes"}), ctx).await
    });

    // The child registers before its first request, and the gate holds that
    // request, so the id is stable here. Bounded so a stuck child fails loudly.
    let id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(id) = agent_kernel::tools::delegate::active_children(0).first().copied() {
                return id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("child never registered a steer channel");
    agent_kernel::tools::delegate::steer_child(0, id, "use the other file".into())
        .expect("running child must accept a steer");

    // Release the held request; the steer is drained before the next one.
    gate.notify_one();
    run.await.unwrap().expect("child must finish");

    let seen = provider.seen_messages.lock().unwrap();
    let last = seen.last().expect("a second request was made");
    assert!(
        last.iter()
            .any(|m| m.contains("The user interrupted: use the other file")),
        "steer missing from the next request: {last:?}"
    );
}

/// A steer to a child that already finished is refused rather than queued, so
/// it cannot sit unread in a channel nobody drains.
#[tokio::test]
async fn steer_to_a_finished_child_is_refused() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("all done");
    let ctx = ctx_with(dir.path(), spawner(provider));
    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(serde_json::json!({"task": "quick"}), ctx)
        .await
        .expect("child must finish");

    assert!(agent_kernel::tools::delegate::active_children(0).is_empty());
    let err = agent_kernel::tools::delegate::steer_child(0, 999, "late".into())
        .expect_err("no such running child");
    assert!(err.contains("no running subagent"), "{err}");
}

/// Writes a project `AGENTS.md` the child would inherit under the default rule.
fn project_rules(dir: &std::path::Path) {
    std::fs::write(dir.join("AGENTS.md"), "RULE-FROM-REPO: run cargo fmt").unwrap();
}

/// The first request's system message, which carries any inherited repo rules.
fn first_system(provider: &ScriptedProvider) -> String {
    provider.seen_messages.lock().unwrap()[0].join("\n")
}

/// A writing child (`readonly` absent) inherits the repo rules by default.
#[tokio::test]
async fn writing_child_inherits_repo_rules_by_default() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    project_rules(dir.path());
    manifest(dir.path(), "builder", "");
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("ok");
    let ctx = ctx_with(dir.path(), spawner(provider.clone()));

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(serde_json::json!({"task": "fix it", "agent": "builder"}), ctx)
        .await
        .expect("writer must run");

    assert!(first_system(&provider).contains("RULE-FROM-REPO"));
}

/// A read-only child does not inherit repo rules by default, so an independent
/// investigation is not steered by them.
#[tokio::test]
async fn readonly_child_does_not_inherit_repo_rules_by_default() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    project_rules(dir.path());
    manifest(dir.path(), "scout", "readonly: true\n");
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("ok");
    let ctx = ctx_with(dir.path(), spawner(provider.clone()));

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(serde_json::json!({"task": "look", "agent": "scout"}), ctx)
        .await
        .expect("scout must run");

    assert!(!first_system(&provider).contains("RULE-FROM-REPO"));
}

/// An explicit `context_files: true` in the manifest overrides the read-only
/// default, and the contract still comes first.
#[tokio::test]
async fn manifest_context_files_overrides_the_readonly_default() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    project_rules(dir.path());
    manifest(dir.path(), "auditor", "readonly: true\ncontext_files: true\n");
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("ok");
    let ctx = ctx_with(dir.path(), spawner(provider.clone()));

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(serde_json::json!({"task": "audit", "agent": "auditor"}), ctx)
        .await
        .expect("auditor must run");

    let system = first_system(&provider);
    assert!(system.contains("RULE-FROM-REPO"), "{system}");
    assert!(
        system.starts_with("You are a delegated subagent"),
        "contract must lead: {system}"
    );
}

/// Every child leaves a readable transcript under `.husk/subagents/`: one JSON
/// message per line, ending with the answer the parent was given.
#[tokio::test]
async fn child_transcript_is_saved_as_jsonl() {
    let _serial = DELEGATE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("the transcript answer");
    let ctx = ctx_with(dir.path(), spawner(provider));

    let spec = ToolRegistry::with_builtins().spec("delegate").unwrap();
    (spec.exec)(serde_json::json!({"task": "record me"}), ctx)
        .await
        .expect("child must finish");

    let files: Vec<_> = std::fs::read_dir(dir.path().join(".husk/subagents"))
        .expect("transcript directory exists")
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");

    let text = std::fs::read_to_string(&files[0]).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is a JSON message"))
        .collect();
    assert!(lines.len() >= 2, "{lines:?}");
    let last = lines.last().unwrap();
    assert!(
        last.to_string().contains("the transcript answer"),
        "{last}"
    );
}
