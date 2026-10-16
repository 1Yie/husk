//! A running turn must survive the actor disappearing before its final save.

use agent_ipc::UiEvent;
use agent_llm::types::{ChatMessage, Role, StreamChunk};
use agent_llm::{BoxStream, LlmProvider, ModelParams};
use agent_store::SessionStore;
use agent_testkit::ScriptedProvider;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn cfg(root: &std::path::Path, provider: Arc<dyn LlmProvider>) -> SessionConfig {
    SessionConfig {
        workspace_root: root.into(),
        paths: agent_paths::AppPaths::for_tests("kernel"),
        provider,
        provider_name: "test".into(),
        model: "test".into(),
        temperature: 0.0,
        permission_mode: "default".into(),
        agent_mode: "build".into(),
        track_dirty: false,
        thinking_level: None,
        thinking_level_map: None,
        context_window: None,
        compact_at: None,
        model_input: vec![],
        model_params: None,
        plugins: None,
        queued_prompts: vec![],
        memory_enabled: false,
        memory_distill: false,
    }
}

struct LiveProvider(Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<StreamChunk>>>);

#[async_trait::async_trait]
impl LlmProvider for LiveProvider {
    fn id(&self) -> &'static str {
        "live-test"
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
        let rx = self.0.lock().unwrap().take().expect("only one request");
        Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|chunk| (Ok(chunk), rx))
        })))
    }
}

async fn stored_where(
    root: &std::path::Path,
    id: i64,
    matches: impl Fn(&[ChatMessage]) -> bool,
) -> Vec<ChatMessage> {
    let store = SessionStore::open(root).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(history) = store.load_history(id) {
                if matches(&history) {
                    return history;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the running conversation was not saved")
}

async fn wait_for_approval(channels: &mut agent_kernel::channels::UiChannels<UiCommand>) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = channels.event_rx.recv().await {
            if matches!(event, UiEvent::ApprovalRequested { .. }) {
                return;
            }
        }
        panic!("the running turn never reached its pending tool");
    })
    .await
    .unwrap();
}

fn pending_write(provider: &ScriptedProvider) {
    provider.push_script(vec![
        StreamChunk::ContentDelta("progress before the crash".into()),
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("write".into()),
            name: Some("apply_patch".into()),
            args_delta: serde_json::json!({
                "patch": "*** Begin Patch\n*** Add File: recovered.txt\n+fixed\n*** End Patch\n"
            })
            .to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]);
}

#[tokio::test]
async fn interrupted_turn_preserves_prompt_reasoning_and_tool_progress() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("previous answer");
    provider.push_script(vec![
        StreamChunk::ReasoningDelta("investigation so far".into()),
        StreamChunk::ContentDelta("I found the cause and will write the fix.".into()),
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("pending-write".into()),
            name: Some("apply_patch".into()),
            args_delta: serde_json::json!({
                "patch": "*** Begin Patch\n*** Add File: recovered.txt\n+fixed\n*** End Patch\n"
            })
            .to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]);
    let (mut actor, mut channels) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    actor
        .handle(UiCommand::Prompt {
            text: "previous task".into(),
        })
        .await;
    let tx = actor.command_sender();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt {
        text: "long running task".into(),
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = channels.event_rx.recv().await {
            if matches!(event, UiEvent::ApprovalRequested { .. }) {
                return;
            }
        }
        panic!("the running turn never reached its pending tool");
    })
    .await
    .unwrap();

    // Read from a fresh store while the actor is still running: recovery must
    // not depend on cancellation, Drop, or a clean application shutdown.
    let recovered = SessionStore::open(dir.path()).unwrap().load_history(id);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let recovered = recovered.expect("the running session must already be on disk");
    assert!(recovered
        .iter()
        .any(|m| m.content.as_deref() == Some("previous answer")));
    assert!(
        recovered
            .iter()
            .any(|m| m.content.as_deref() == Some("long running task")),
        "the in-flight task was lost when the actor disappeared"
    );
    let draft = recovered
        .iter()
        .find(|m| m.content.as_deref() == Some("I found the cause and will write the fix."))
        .expect("the assistant's progress must survive an interrupted turn");
    assert_eq!(draft.reasoning.as_deref(), Some("investigation so far"));
    assert!(recovered
        .iter()
        .any(|m| m.content.as_deref()
            == Some("上次任务因会话意外中断而停止，已恢复到最后保存的进度。")));
    assert_eq!(draft.tool_calls.as_ref().unwrap()[0].id, "pending-write");
    let pending = recovered
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("pending-write"))
        .unwrap();
    assert_eq!(pending.is_error, Some(true));
    assert!(pending.content.as_deref().unwrap().contains("执行结果未知"));
    assert!(!dir.path().join("recovered.txt").exists());
}

#[tokio::test]
async fn idle_stream_saves_the_latest_draft_and_resumes_without_partial_tool_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let (chunks, rx) = tokio::sync::mpsc::unbounded_channel();
    let provider = Arc::new(LiveProvider(Mutex::new(Some(rx))));
    let (mut actor, _) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    let tx = actor.command_sender();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt {
        text: "streaming task".into(),
    })
    .await
    .unwrap();
    chunks
        .send(StreamChunk::ContentDelta("first".into()))
        .unwrap();
    stored_where(dir.path(), id, |h| {
        h.iter().any(|m| m.content.as_deref() == Some("first"))
    })
    .await;

    // This tail arrives inside the throttle window, then the provider stalls.
    // A timer, rather than a later token or Drop, must flush it to disk.
    chunks
        .send(StreamChunk::ReasoningDelta("still investigating".into()))
        .unwrap();
    chunks
        .send(StreamChunk::ContentDelta(" and latest tail".into()))
        .unwrap();
    chunks
        .send(StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("half-call".into()),
            name: Some("apply_patch".into()),
            args_delta: "{\"patch\":".into(),
        })
        .unwrap();
    let recovered = stored_where(dir.path(), id, |h| {
        h.iter().any(|m| {
            m.content.as_deref() == Some("first and latest tail")
                && m.reasoning.as_deref() == Some("still investigating")
        })
    })
    .await;
    assert!(recovered.iter().all(|m| m.tool_calls.is_none()));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let next = Arc::new(ScriptedProvider::new());
    next.script_text("continued successfully");
    let (mut resumed, _) = SessionActor::resume(cfg(dir.path(), next), id, recovered);
    resumed
        .handle(UiCommand::Prompt {
            text: "continue".into(),
        })
        .await;
    let history = store.load_history(id).unwrap();
    assert!(history
        .iter()
        .any(|m| m.content.as_deref() == Some("first and latest tail")));
    assert!(history
        .iter()
        .any(|m| m.content.as_deref() == Some("continued successfully")));
    assert_eq!(
        store
            .list()
            .into_iter()
            .find(|m| m.id == id)
            .unwrap()
            .preview,
        "continued successfully"
    );
}

#[tokio::test]
async fn reasoning_only_draft_survives_resume_and_the_next_save() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let (chunks, rx) = tokio::sync::mpsc::unbounded_channel();
    let provider = Arc::new(LiveProvider(Mutex::new(Some(rx))));
    let (mut actor, _) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    let tx = actor.command_sender();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt {
        text: "think first".into(),
    })
    .await
    .unwrap();
    chunks
        .send(StreamChunk::ReasoningDelta(
            "a useful partial thought".into(),
        ))
        .unwrap();
    let recovered = stored_where(dir.path(), id, |h| {
        h.iter()
            .any(|m| m.reasoning.as_deref() == Some("a useful partial thought"))
    })
    .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("resumed answer");
    let (mut actor, _) = SessionActor::resume(cfg(dir.path(), provider), id, recovered);
    assert!(actor
        .history()
        .iter()
        .any(|m| m.reasoning.as_deref() == Some("a useful partial thought")));
    actor
        .handle(UiCommand::Prompt {
            text: "continue".into(),
        })
        .await;
    assert!(store
        .load_history(id)
        .unwrap()
        .iter()
        .any(|m| m.content.as_deref() == Some("resumed answer")));
}

#[tokio::test]
async fn completed_same_named_tools_keep_their_own_results_before_the_next_model_round() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("source.txt"),
        "valuable investigation result\n",
    )
    .unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let provider = Arc::new(ScriptedProvider::new());
    provider.push_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0, id: Some("missing-read".into()), name: Some("smart_read".into()),
            args_delta: serde_json::json!({"path":"missing.txt", "mode":"range"}).to_string(),
        },
        StreamChunk::ToolCallDelta {
            slot: 1, id: Some("read".into()), name: Some("smart_read".into()),
            args_delta: serde_json::json!({"path":"source.txt", "mode":"range"}).to_string(),
        },
        StreamChunk::ToolCallDelta {
            slot: 2, id: Some("write".into()), name: Some("apply_patch".into()),
            args_delta: serde_json::json!({"patch":"*** Begin Patch\n*** Add File: recovered.txt\n+fixed\n*** End Patch\n"}).to_string(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]);
    let (mut actor, mut channels) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    let tx = actor.command_sender();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt {
        text: "read then write".into(),
    })
    .await
    .unwrap();
    wait_for_approval(&mut channels).await;
    let history = SessionStore::open(dir.path())
        .unwrap()
        .load_history(id)
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let missing = history
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("missing-read"))
        .unwrap();
    assert_eq!(missing.is_error, Some(true));
    assert!(!missing
        .content
        .as_deref()
        .unwrap()
        .contains("valuable investigation result"));
    let read = history
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("read"))
        .unwrap();
    assert_ne!(read.is_error, Some(true));
    assert!(read
        .content
        .as_deref()
        .unwrap()
        .contains("valuable investigation result"));
    let write = history
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("write"))
        .unwrap();
    assert_eq!(write.is_error, Some(true));
    assert!(!dir.path().join("recovered.txt").exists());
}

#[tokio::test]
async fn completed_turn_replaces_the_recovery_draft_and_interruption_notice() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("complete answer");
    let (mut actor, _) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    actor
        .handle(UiCommand::Prompt {
            text: "complete task".into(),
        })
        .await;
    let history = store.load_history(id).unwrap();
    assert_eq!(history, actor.history());
    assert_eq!(
        history.iter().filter(|m| m.role == Role::Assistant).count(),
        1
    );
    assert_eq!(
        history.last().unwrap().content.as_deref(),
        Some("complete answer")
    );
    assert!(!history.iter().any(|m| m
        .content
        .as_deref()
        .is_some_and(|s| s.contains("会话意外中断"))));
}

#[tokio::test]
async fn repaired_legacy_history_keeps_the_persisted_revision_for_the_next_save() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let history = vec![
        ChatMessage::system("test system"),
        ChatMessage::user("old task"),
        ChatMessage::assistant(""),
        ChatMessage::tool_result("orphan-call", "orphaned output"),
    ];
    store.snapshot(id, &history).unwrap();
    let provider = Arc::new(ScriptedProvider::new());
    provider.script_text("saved after repair");
    let (mut actor, _) = SessionActor::resume(cfg(dir.path(), provider), id, history);
    assert_eq!(actor.history().len(), 2);
    actor
        .handle(UiCommand::Prompt {
            text: "new task".into(),
        })
        .await;
    let history = store.load_history(id).unwrap();
    assert_eq!(history, actor.history());
    assert_eq!(
        history.last().unwrap().content.as_deref(),
        Some("saved after repair")
    );
}

#[tokio::test]
async fn another_window_commit_stops_saving_and_reports_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::open(dir.path()).unwrap();
    let id = store.next_id();
    let (chunks, rx) = tokio::sync::mpsc::unbounded_channel();
    let provider = Arc::new(LiveProvider(Mutex::new(Some(rx))));
    let (mut actor, mut channels) = SessionActor::spawn_with_id(cfg(dir.path(), provider), id);
    let tx = actor.command_sender();
    let task = tokio::spawn(async move { actor.run().await });
    tx.send(UiCommand::Prompt {
        text: "contended task".into(),
    })
    .await
    .unwrap();
    chunks
        .send(StreamChunk::ContentDelta("before".into()))
        .unwrap();
    let saved = stored_where(dir.path(), id, |h| {
        h.iter().any(|m| m.content.as_deref() == Some("before"))
    })
    .await;

    // Another window commits on top of the checkpoint's revision.
    let meta = store.list().into_iter().find(|m| m.id == id).unwrap();
    let mut other = saved.clone();
    other.push(ChatMessage::assistant("written by another window"));
    store
        .commit_turn_checked(&other, meta, agent_store::SessionStore::history_revision_of(&saved))
        .unwrap();

    // Later progress would overwrite that window, so it must never land.
    chunks
        .send(StreamChunk::ContentDelta(" and after".into()))
        .unwrap();
    let notices = tokio::time::timeout(Duration::from_secs(3), async {
        let mut notices = 0;
        while let Some(event) = channels.event_rx.recv().await {
            if matches!(&event, UiEvent::SystemMessage(m) if m.contains("保存运行中的会话失败")) {
                notices += 1;
                // Keep draining briefly: a repeat notice would show up here.
                let extra = tokio::time::timeout(Duration::from_millis(800), async {
                    while let Some(event) = channels.event_rx.recv().await {
                        if matches!(&event, UiEvent::SystemMessage(m) if m.contains("保存运行中的会话失败")) {
                            return 1;
                        }
                    }
                    0
                })
                .await
                .unwrap_or(0);
                return notices + extra;
            }
        }
        notices
    })
    .await
    .expect("the conflict was never reported");
    assert_eq!(notices, 1, "the failure must be reported exactly once");
    task.abort();
    let _ = task.await;
    let history = store.load_history(id).unwrap();
    assert_eq!(history, other);
}

// Invoked in a separate process by the SIGKILL test below. In an ordinary
// suite run this helper has no side effects.
#[cfg(unix)]
#[test]
fn crash_writer_process() {
    let Some(root) = std::env::var_os("HUSK_CRASH_WRITER_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let provider = Arc::new(ScriptedProvider::new());
        pending_write(&provider);
        let (mut actor, mut channels) = SessionActor::spawn_with_id(cfg(&root, provider), 1);
        let tx = actor.command_sender();
        tokio::spawn(async move { actor.run().await });
        tx.send(UiCommand::Prompt {
            text: "task killed by the OS".into(),
        })
        .await
        .unwrap();
        wait_for_approval(&mut channels).await;
        std::fs::write(root.join("ready"), b"ready").unwrap();
        std::future::pending::<()>().await;
    });
}

#[cfg(unix)]
#[test]
fn sigkill_preserves_the_running_turn_without_drop_or_shutdown_handlers() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session-database");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_writer_process", "--nocapture"])
        .env("HUSK_CRASH_WRITER_ROOT", dir.path())
        .env("AGENT_SESSION_DB", &database)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !dir.path().join("ready").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("crash writer exited before reaching the running turn: {status}");
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("crash writer did not reach the pending tool");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    assert_eq!(child.wait().unwrap().signal(), Some(9));
    let history = SessionStore::open_at(dir.path(), &database)
        .unwrap()
        .load_history(1)
        .unwrap();
    assert!(history
        .iter()
        .any(|m| m.role == Role::User && m.content.as_deref() == Some("task killed by the OS")));
    assert!(history
        .iter()
        .any(|m| m.content.as_deref() == Some("progress before the crash")));
    assert!(history
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("write") && m.is_error == Some(true)));
}
