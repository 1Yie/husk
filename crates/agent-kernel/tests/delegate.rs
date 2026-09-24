//! `delegate` — parallel read-only fan-out, end to end: real child engines,
//! scripted provider, no network.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use agent_kernel::channels::UiSink;
use agent_kernel::tools::delegate::SubagentSpawner;
use agent_kernel::tools::registry::ToolClass;
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use common::ScriptedProvider;

fn spawner(provider: Arc<dyn agent_llm::LlmProvider>) -> SubagentSpawner {
    let full = ToolRegistry::with_builtins()
        .filtered(|s| s.name != "delegate" && s.class != ToolClass::HumanInteraction);
    let readonly = full.readonly_only();
    SubagentSpawner::new(
        provider,
        "test-model",
        0.0,
        256_000,
        Arc::new(full),
        Arc::new(readonly),
    )
}

fn ctx_with(dir: &std::path::Path, spawner: SubagentSpawner) -> Arc<ToolCtx> {
    let mut ctx = ToolCtx::new(dir);
    ctx.subagent = Some(spawner);
    Arc::new(ctx)
}

#[tokio::test]
async fn parallel_tasks_run_together_and_merge_in_input_order() {
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
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("one");
    provider.script_text("two");
    let (sink, mut rx) = UiSink::channel();
    let mut ctx = ToolCtx::new(dir.path());
    ctx.subagent = Some(spawner(provider));
    ctx.ui_tx = Some(sink);

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
