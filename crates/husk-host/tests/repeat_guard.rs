//! The loop guard through a real engine: a model that keeps re-reading the
//! same file is stopped before the call runs again, and the refusal is what it
//! sees in its tool result — not a silent truncation.

use std::sync::Arc;

use agent_capabilities::WorkspaceCtx;
use agent_kernel::engine::{Engine, EngineIo};
use agent_kernel::hooks::{AgentHook, HookChain};
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use agent_llm::types::{ChatMessage, StreamChunk};
use agent_testkit::ScriptedProvider;

fn read_round(round: usize, path: &str) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some(format!("read-{round}")),
            name: Some("smart_read".into()),
            args_delta: serde_json::json!({ "path": path }).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]
}

#[tokio::test]
async fn a_repeated_identical_call_is_refused_before_it_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "pub fn a() {}\n").unwrap();
    let path = root.join("lib.rs").to_string_lossy().into_owned();

    let provider = Arc::new(ScriptedProvider::new());
    // Six identical reads. The first two differ (the observation ledger adds
    // its "identical to the earlier output" note from the second on), and the
    // rest return byte-identical results — a loop.
    for round in 0..6 {
        provider.push_script(read_round(round, &path));
    }
    provider.script_text("stopped");

    let guard = Arc::new(husk_host::repeat_guard::RepeatGuard::default());
    let local: Vec<Arc<dyn AgentHook>> = vec![guard];
    let ctx = Arc::new(ToolCtx::new().with_workspace(&root));
    let registry: Arc<ToolRegistry> = Arc::new(husk_host::session::base_registries().0);
    let mut engine = Engine::new(provider, registry, ctx, "test", 0.0);
    engine.set_hooks(HookChain::layered(local, &HookChain::new()));

    let (ui_tx, _ui_rx) = agent_kernel::channels::UiSink::channel();
    let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel(4);
    let mut io = EngineIo {
        ui_tx,
        steer_rx,
        notice_rx: tokio::sync::mpsc::channel(4).1,
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let mut history = vec![ChatMessage::system("test")];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        engine.run_turn(&mut io, &mut history, "read it".into()),
    )
    .await
    .expect("the turn must finish instead of looping")
    .expect("turn");

    let results: Vec<String> = history
        .iter()
        .filter(|m| m.role == agent_llm::Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    assert!(
        results.len() >= 6,
        "every call still gets a result row: {}",
        results.len()
    );
    // Four identical results is what it takes to establish that nothing is
    // moving; from then on the call is refused every time it is repeated.
    let served = results.iter().filter(|r| r.contains("pub fn a()")).count();
    assert_eq!(served, 4, "the reads before the loop were served: {results:#?}");
    let refusals = results.iter().filter(|r| r.contains("已拒绝")).count();
    assert_eq!(refusals, 2, "the repeats are stopped: {results:#?}");
}
