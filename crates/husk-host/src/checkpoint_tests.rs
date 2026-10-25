use super::*;
use fs2::FileExt as _;
use std::fs::OpenOptions;
use std::sync::atomic::AtomicBool;

fn setup() -> (
    tempfile::TempDir,
    Arc<SessionStore>,
    Arc<TurnCheckpoint>,
    UiReceiver,
) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("database");
    let store = Arc::new(SessionStore::open_at(dir.path(), &database).unwrap());
    let (ui, events) = UiSink::channel();
    let (forward, output) = UiSink::channel();
    let checkpoint = TurnCheckpoint::new(store.clone(), 0, ui, events, forward);
    checkpoint.start();
    (dir, store, checkpoint, output)
}

fn meta() -> SessionMeta {
    SessionMeta {
        id: 1,
        title: "task".into(),
        ..Default::default()
    }
}

fn call(id: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read".into(),
        arguments: arguments.into(),
    }
}

#[tokio::test]
async fn live_display_hides_recovery_only_rows_until_the_turn_commits() {
    let (_dir, store, checkpoint, _output) = setup();
    let mut assistant = ChatMessage::assistant("");
    assistant.tool_calls = Some(vec![call("pending", "{}")]);
    checkpoint.begin(&[ChatMessage::system("system"), assistant], "task", meta());
    checkpoint.drain().await;

    // The resumed engine needs a paired result and the interrupted notice.
    let stored = store.load_history(1).unwrap();
    assert!(stored
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("pending") && m.is_error == Some(true)));
    assert!(stored
        .iter()
        .any(|m| m.content.as_deref() == Some(INTERRUPTED_TURN)));

    // The live trace must show only the draft — no synthetic recovery rows.
    let (page, total, _, _) = store.load_history_page(1, None, 50).unwrap();
    // system + assistant + the prompt `begin` appended; the placeholder and
    // the interrupted notice are recovery-only.
    assert_eq!(total, 3);
    assert!(page
        .iter()
        .all(|m| m.tool_call_id.as_deref() != Some("pending")));
    assert!(page
        .iter()
        .all(|m| m.content.as_deref() != Some(INTERRUPTED_TURN)));
}

#[tokio::test]
async fn usage_events_forward_the_cache_write_counter() {
    let (_dir, _store, checkpoint, mut output) = setup();
    let observer = Arc::new(agent_llm::UsageObserver::default());
    observer.set_cache_write(42);
    checkpoint.set_usage_observer(observer);

    checkpoint
        .ui
        .send(agent_ipc::UiEvent::Usage {
            prompt_tokens: 100,
            completion_tokens: 20,
            context_window: 1000,
            cached_tokens: 30,
        })
        .unwrap();

    let forwarded = tokio::time::timeout(Duration::from_secs(2), output.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(forwarded, agent_ipc::UiEvent::Usage { .. }));
    let extra = tokio::time::timeout(Duration::from_secs(2), output.recv())
        .await
        .unwrap()
        .unwrap();
    match extra {
        agent_ipc::UiEvent::Custom { kind, payload } => {
            assert_eq!(kind, USAGE_CACHE_WRITE);
            assert_eq!(payload.as_u64(), Some(42));
        }
        other => panic!("expected a cache-write custom event, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_busy_file_lock_does_not_block_the_async_thread() {
    let (dir, store, checkpoint, _output) = setup();
    let lock_path = dir
        .path()
        .join("database")
        .join(store.dir_name().unwrap())
        .join("1.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
        .unwrap();
    file.lock_exclusive().unwrap();
    let released = Arc::new(AtomicBool::new(false));
    let release_flag = released.clone();
    let (release, held) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        // The timeout bounds a regression: synchronous checkpointing would
        // wait here before returning from begin(), instead of yielding.
        let _ = held.recv_timeout(Duration::from_secs(1));
        release_flag.store(true, Ordering::Release);
        fs2::FileExt::unlock(&file).unwrap();
    });
    checkpoint.begin(&[ChatMessage::system("system")], "task", meta());
    assert!(
        !released.load(Ordering::Acquire),
        "begin blocked on the file lock"
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !released.load(Ordering::Acquire),
        "the async timer could not run while the writer was busy"
    );
    release.send(()).unwrap();
    checkpoint.drain().await;
    holder.join().unwrap();
    assert!(store
        .load_history(1)
        .unwrap()
        .iter()
        .any(|m| m.content.as_deref() == Some("task")));
}

#[tokio::test]
async fn a_transient_write_failure_retries_without_another_model_chunk() {
    let (dir, store, checkpoint, mut output) = setup();
    let recovery = dir
        .path()
        .join("database")
        .join(store.dir_name().unwrap())
        .join("1.recovery.json");
    std::fs::create_dir(&recovery).unwrap();
    checkpoint.begin(&[ChatMessage::system("system")], "task", meta());
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(event) = output.recv().await {
            if matches!(event, agent_ipc::UiEvent::SystemMessage(ref text) if text.contains("保存运行中的会话失败")) {
                break;
            }
        }
    }).await.unwrap();
    std::fs::remove_dir(&recovery).unwrap();
    checkpoint.drain().await;
    let history = store
        .load_history(1)
        .expect("the background retry persisted an idle turn");
    assert!(history.iter().any(|m| m.content.as_deref() == Some("task")));
    let state = checkpoint.state.lock().unwrap();
    assert_eq!(state.failures, 0);
    assert!(!state.failed);
}

#[tokio::test]
async fn a_different_system_context_cannot_replace_user_history_even_with_the_same_first_line() {
    let (_dir, store, checkpoint, _output) = setup();
    checkpoint.begin(
        &[ChatMessage::system("shared heading\nuser context")],
        "task",
        meta(),
    );
    checkpoint.drain().await;
    let revision = checkpoint.revision();
    assert!(checkpoint
        .request(
            &[
                ChatMessage::system("shared heading\ncompaction context"),
                ChatMessage::user("summarize this"),
            ],
            true
        )
        .is_none());
    checkpoint.drain().await;
    assert_eq!(checkpoint.revision(), revision);
    assert!(!store
        .load_history(1)
        .unwrap()
        .iter()
        .any(|m| m.content.as_deref() == Some("summarize this")));
}

#[test]
fn identical_flattened_text_is_restored_by_result_ids() {
    let prefix = "x".repeat(300);
    let mut first = ChatMessage::assistant("");
    first.ts = Some(1);
    first.tool_calls = Some(vec![call("first-id", &format!("{prefix}first{prefix}"))]);
    let mut second = ChatMessage::assistant("");
    second.ts = Some(1);
    second.tool_calls = Some(vec![call("second-id", &format!("{prefix}other{prefix}"))]);
    let known = vec![first.clone(), second.clone()];
    let mut wire = vec![
        second.clone(),
        ChatMessage::tool_result("second-id", "two"),
        first.clone(),
        ChatMessage::tool_result("first-id", "one"),
    ];
    agent_kernel::compaction::flatten_tool_calls_for_wire(&mut wire);
    assert_eq!(
        wire[0].content, wire[2].content,
        "the legacy text match was ambiguous"
    );
    restore_tool_calls(&mut wire, &known).unwrap();
    assert_eq!(wire[0].tool_calls, second.tool_calls);
    assert_eq!(wire[2].tool_calls, first.tool_calls);
}

struct Veto;

#[async_trait::async_trait]
impl AgentHook for Veto {
    fn id(&self) -> &str {
        "test-veto"
    }
    async fn before_tool_execute(&self, _call: &mut ToolCall) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("vetoed"))
    }
}

#[tokio::test]
async fn exact_dispatch_ids_cover_vetoes_and_shared_plugin_reload() {
    let (_dir, store, checkpoint, _output) = setup();
    let mut assistant = ChatMessage::assistant("");
    assistant.tool_calls = Some(vec![call("one", "{}"), call("two", "{}")]);
    checkpoint.begin(&[ChatMessage::system("system"), assistant], "task", meta());
    let shared = HookChain::new();
    shared.replace(vec![Arc::new(Veto)]);
    let hooks = checkpoint.hooks(shared.clone());
    assert_eq!(shared.ids(), vec!["test-veto"]);
    assert_eq!(
        hooks
            .run_before_tool(&mut call("one", "{}"))
            .await
            .unwrap_err(),
        "vetoed"
    );
    checkpoint.observe(&agent_ipc::UiEvent::ToolCallFinished {
        name: "read".into(),
        ok: false,
        content: "vetoed".into(),
        ui_type: None,
        parent: None,
        child_id: None,
    });
    shared.replace(vec![]);
    hooks.run_before_tool(&mut call("two", "{}")).await.unwrap();
    checkpoint.observe(&agent_ipc::UiEvent::ToolCallFinished {
        name: "read".into(),
        ok: true,
        content: "second output".into(),
        ui_type: None,
        parent: None,
        child_id: None,
    });
    checkpoint.drain().await;
    let history = store.load_history(1).unwrap();
    assert!(history
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("one")
            && m.content.as_deref() == Some("vetoed")
            && m.is_error == Some(true)));
    assert!(history
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("two")
            && m.content.as_deref() == Some("second output")
            && m.is_error == Some(false)));
}

struct Slow;

#[async_trait::async_trait]
impl AgentHook for Slow {
    fn id(&self) -> &str {
        "test-slow"
    }
    async fn before_tool_execute(&self, _call: &mut ToolCall) -> anyhow::Result<()> {
        // Two of these exceed one hook timeout; each alone stays under it.
        tokio::time::sleep(agent_kernel::hooks::HOOK_TIMEOUT / 2 + Duration::from_millis(100))
            .await;
        Ok(())
    }
}

/// The recovery layer must not fold the plugin chain into one timed hook:
/// slow plugins ahead of a veto would then exhaust a shared budget and the
/// timeout would degrade the whole chain to allow.
#[tokio::test]
async fn slow_plugins_cannot_push_a_later_veto_past_a_shared_timeout() {
    let (_dir, _store, checkpoint, _output) = setup();
    let mut assistant = ChatMessage::assistant("");
    assistant.tool_calls = Some(vec![call("one", "{}")]);
    checkpoint.begin(&[ChatMessage::system("system"), assistant], "task", meta());
    let shared = HookChain::new();
    shared.replace(vec![Arc::new(Slow), Arc::new(Slow), Arc::new(Veto)]);
    let hooks = checkpoint.hooks(shared);
    assert_eq!(
        hooks
            .run_before_tool(&mut call("one", "{}"))
            .await
            .unwrap_err(),
        "vetoed"
    );
}

/// A streamed write keeps the turn-start stamp `begin` was given. The sidebar
/// orders sessions by it; re-stamping per write made two running sessions
/// overtake each other on every flush.
#[tokio::test]
async fn streamed_writes_keep_the_turn_start_stamp() {
    let (_dir, store, checkpoint, _output) = setup();
    let mut started = meta();
    started.updated_at = 42;
    checkpoint.begin(&[ChatMessage::system("system")], "task", started);
    checkpoint.drain().await;
    assert_eq!(store.meta(1).unwrap().updated_at, 42);
}

/// A provider that sits silent behind keep-alives and then answers.
struct SilentThenAnswer;

#[async_trait::async_trait]
impl LlmProvider for SilentThenAnswer {
    fn id(&self) -> &'static str {
        "silent"
    }

    async fn chat_stream(
        &self,
        _model: &str,
        _messages: &[ChatMessage],
        _tools: Option<serde_json::Value>,
        _temperature: f32,
        _reasoning_effort: Option<&str>,
        _params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        Ok(Box::pin(futures::stream::iter([
            Ok(StreamChunk::Keepalive),
            Ok(StreamChunk::Keepalive),
            Ok(StreamChunk::ContentDelta("the answer".into())),
            Ok(StreamChunk::Done {
                prompt_tokens: None,
                completion_tokens: None,
                cached_tokens: None,
            }),
        ])))
    }
}

/// A keep-alive carries no content: it passes through untouched and leaves no
/// empty assistant row in the draft, and the answer behind it still lands.
#[tokio::test]
async fn keepalive_is_forwarded_but_never_drafted() {
    use futures::StreamExt;

    let (_dir, store, checkpoint, _output) = setup();
    let system = ChatMessage::system("system");
    let request = vec![system.clone(), ChatMessage::user("task")];
    checkpoint.begin(std::slice::from_ref(&system), "task", meta());
    checkpoint.drain().await;
    let provider = checkpoint.wrap(Arc::new(SilentThenAnswer));
    let mut stream = provider
        .chat_stream("m", &request, None, 0.0, None, &ModelParams::EMPTY)
        .await
        .unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, StreamChunk::Keepalive));
    checkpoint.drain().await;
    let before = store.load_history(1).unwrap();
    assert!(
        before.iter().all(|m| m.role != Role::Assistant),
        "a keep-alive wrote an empty assistant draft: {before:?}"
    );

    while stream.next().await.is_some() {}
    drop(stream);
    checkpoint.drain().await;
    let after = store.load_history(1).unwrap();
    assert!(
        after
            .iter()
            .any(|m| m.role == Role::Assistant && m.content.as_deref() == Some("the answer")),
        "the answer never reached the draft: {after:?}"
    );
}
