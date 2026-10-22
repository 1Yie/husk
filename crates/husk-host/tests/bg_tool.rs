//! End-to-end: bg_run start → wait/list/logs → kill, through the real
//! sandbox backend (bwrap on Linux).
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use agent_capabilities::{SandboxCtx, WorkspaceCtx};
use agent_kernel::tools::registry::ToolCtx;
use serde_json::json;

fn ctx_with_sandbox(root: &std::path::Path) -> (ToolCtx, Arc<agent_tools_shell::bg::BgRegistry>) {
    let (detected, _loud) = agent_sandbox::detect_backend();
    let mut ctx = ToolCtx::new().with_workspace(root);
    ctx.install_sandbox(Arc::from(detected));
    let reg = agent_tools_shell::bg::install_registry(&mut ctx);
    (ctx, reg)
}

fn call(args: serde_json::Value, ctx: Arc<ToolCtx>, rt: &tokio::runtime::Runtime) -> String {
    let spec = agent_tools_shell::bg::spec::<ToolCtx>();
    rt.block_on((spec.exec)(args, ctx))
        .map(|r| r.content)
        .unwrap_or_else(|e| format!("ERR {e:?}"))
}

fn id_of(out: &str) -> String {
    out.split_whitespace()
        .find(|t| t.starts_with("id="))
        .expect("run returns an id")
        .trim_start_matches("id=")
        .to_string()
}

#[test]
fn background_job_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _reg) = ctx_with_sandbox(dir.path());
    let ctx = Arc::new(ctx);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let out = call(
        json!({"action":"run","command":"echo start; sleep 0.2; echo done"}),
        ctx.clone(),
        &rt,
    );
    assert!(out.contains("bg-"), "spawn should return an id: {out}");
    let id = id_of(&out);

    let out = call(
        json!({"action":"wait","id":id,"timeout_secs":10}),
        ctx.clone(),
        &rt,
    );
    assert!(out.contains("exited(0)"), "wait should report exit: {out}");
    assert!(out.contains("done"), "wait should include the tail: {out}");

    let out = call(json!({"action":"list"}), ctx.clone(), &rt);
    assert!(out.contains(&id), "list should contain {id}: {out}");

    let out = call(json!({"action":"logs","id":id}), ctx.clone(), &rt);
    assert!(out.contains("start"), "logs should have output: {out}");

    let out = call(json!({"action":"remove","id":id}), ctx.clone(), &rt);
    assert!(out.contains("removed"), "remove should report: {out}");
}

#[test]
fn background_job_pause_resume_kill() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _reg) = ctx_with_sandbox(dir.path());
    let ctx = Arc::new(ctx);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let out = call(
        json!({"action":"run","command":"sleep 30"}),
        ctx.clone(),
        &rt,
    );
    let id = id_of(&out);

    let out = call(json!({"action":"pause","id":id}), ctx.clone(), &rt);
    assert!(out.contains("paused"), "pause should report: {out}");
    let out = call(json!({"action":"status","id":id}), ctx.clone(), &rt);
    assert!(out.contains("paused"), "status should say paused: {out}");

    let out = call(json!({"action":"resume","id":id}), ctx.clone(), &rt);
    assert!(out.contains("resumed"), "resume should report: {out}");

    let out = call(json!({"action":"kill","id":id}), ctx.clone(), &rt);
    assert!(out.contains("kill sent"), "kill should report: {out}");

    let mut status = String::new();
    for _ in 0..50 {
        status = call(json!({"action":"status","id":id}), ctx.clone(), &rt);
        if !status.contains("running") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!status.contains("running"), "job still running: {status}");
    assert!(
        status.contains("killed("),
        "a killed job reports the signal, not an exit code: {status}"
    );
}

/// A job that outlives `max_secs` is killed by its own monitor, so a runaway
/// build cannot hold a slot for the rest of the session.
#[test]
fn background_job_timeout_kills_it() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _reg) = ctx_with_sandbox(dir.path());
    let ctx = Arc::new(ctx);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let out = call(
        json!({"action":"run","command":"sleep 60","max_secs":1}),
        ctx.clone(),
        &rt,
    );
    let id = id_of(&out);

    let out = call(
        json!({"action":"wait","id":id,"timeout_secs":15}),
        ctx.clone(),
        &rt,
    );
    assert!(out.contains("timeout"), "the cap must kill the job: {out}");
}

/// The registry must forget a reaped pid: keeping it would let `Drop` signal
/// whatever process inherited the number.
#[test]
fn a_finished_job_releases_its_pid() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _reg) = ctx_with_sandbox(dir.path());
    let ctx = Arc::new(ctx);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let out = call(json!({"action":"run","command":"true"}), ctx.clone(), &rt);
    let id = id_of(&out);
    let out = call(
        json!({"action":"wait","id":id,"timeout_secs":15}),
        ctx.clone(),
        &rt,
    );
    assert!(out.contains("exited(0)"), "{out}");

    let out = call(json!({"action":"status","id":id}), ctx, &rt);
    assert!(
        out.contains("pid=-"),
        "a finished job must not still claim a pid: {out}"
    );
}

#[test]
fn pause_freezes_the_process_tree() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, _reg) = ctx_with_sandbox(dir.path());
    let ctx = Arc::new(ctx);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ticks = dir.path().join("ticks.txt");
    let command = format!(
        "i=0; while [ $i -lt 200 ]; do echo tick >> {}; i=$((i+1)); sleep 0.1; done",
        ticks.display()
    );

    let out = call(json!({"action":"run","command":command}), ctx.clone(), &rt);
    let id = id_of(&out);
    std::thread::sleep(Duration::from_millis(700));
    let before = std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
    assert!(before > 0, "job should be writing before pause");

    let out = call(json!({"action":"pause","id":id}), ctx.clone(), &rt);
    assert!(out.contains("paused"), "pause should report: {out}");

    // Let any write already in flight land, then snapshot the frozen size.
    std::thread::sleep(Duration::from_millis(300));
    let frozen = std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
    std::thread::sleep(Duration::from_millis(700));
    let after = std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
    assert_eq!(
        after, frozen,
        "paused job kept writing: {frozen} -> {after}"
    );

    let out = call(json!({"action":"resume","id":id}), ctx.clone(), &rt);
    assert!(out.contains("resumed"), "resume should report: {out}");
    std::thread::sleep(Duration::from_millis(800));
    let resumed = std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
    assert!(
        resumed > after,
        "resumed job did not write: {after} -> {resumed}"
    );

    call(json!({"action":"kill","id":id}), ctx, &rt);
}

#[test]
fn completion_notice_is_pushed() {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, reg) = ctx_with_sandbox(dir.path());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    reg.set_notifier(move |text| {
        let _ = tx.send(text);
    });
    let ctx = Arc::new(ctx);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let out = call(
        json!({"action":"run","command":"echo finished"}),
        ctx.clone(),
        &rt,
    );
    let id = id_of(&out);

    let notice = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(10), rx.recv()).await })
        .expect("completion notice timeout")
        .expect("completion notice");
    assert!(notice.contains(&id), "notice should name {id}: {notice}");
    assert!(
        notice.contains("exited(0)"),
        "notice should carry the exit: {notice}"
    );
    assert!(
        notice.contains("finished"),
        "notice should carry the tail: {notice}"
    );
}

/// The session mirrors its jobs to the UI as full `husk:bg_jobs` snapshots,
/// and a finished job reaches the UI only that way — never as a system line
/// carrying the notice the model gets.
#[test]
fn the_session_streams_its_jobs_to_the_ui() {
    use agent_llm::types::StreamChunk;
    use husk_host::command::UiCommand;
    use husk_host::session::{SessionActor, SessionConfig, BG_JOBS};

    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(agent_testkit::ScriptedProvider::new().with_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some("bg_run".into()),
            args_delta: json!({"action":"run","command":"echo streamed"}).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]));
    provider.script_text("started");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, mut channels) = rt.block_on(async {
        SessionActor::spawn(SessionConfig {
            workspace_root: dir.path().to_path_buf(),
            paths: agent_paths::AppPaths::for_tests("kernel"),
            provider: provider.clone(),
            model: "test-model".into(),
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
            memory_enabled: false,
            memory_distill: false,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        })
    });
    let reg = actor.bg_registry();
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt { text: "go".into() }))
        .unwrap();

    // Read events until a snapshot shows the job finished.
    let mut snapshots = Vec::new();
    let mut system_lines = Vec::new();
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(ev) = channels.event_rx.recv().await {
                match ev {
                    agent_ipc::UiEvent::Custom { kind, payload } if kind == BG_JOBS => {
                        let done = payload["jobs"][0]["state"]["kind"] == "done";
                        snapshots.push(payload);
                        if done {
                            break;
                        }
                    }
                    agent_ipc::UiEvent::SystemMessage(text) => system_lines.push(text),
                    _ => {}
                }
            }
        })
        .await
        .expect("a finished-job snapshot");
    });

    let last = snapshots.last().unwrap();
    let job = &last["jobs"][0];
    assert_eq!(job["command"], "echo streamed", "{last}");
    assert_eq!(job["state"]["code"], 0, "{last}");
    assert_eq!(job["state"]["reason"], "exited", "{last}");
    assert_eq!(job["has_own_sink"], true, "the session's own job: {last}");
    assert!(
        system_lines.iter().all(|l| !l.contains("background task")),
        "the notice must not reach the UI as a system line: {system_lines:?}"
    );
    // The handle the manager hands out is the one the feed mirrors.
    assert_eq!(reg.jobs().len(), 1);
    assert!(reg
        .tail(job["id"].as_str().unwrap(), 5)
        .unwrap()
        .contains("streamed"));
}

/// A job that finishes after the turn ended wakes the session on its own: the
/// model gets a second turn with the completion in front of it, and no user
/// prompt opens it.
#[test]
fn an_idle_session_wakes_when_its_job_finishes() {
    use agent_llm::types::StreamChunk;
    use husk_host::command::UiCommand;
    use husk_host::session::{SessionActor, SessionConfig};

    let dir = tempfile::tempdir().unwrap();
    let provider =
        Arc::new(agent_testkit::ScriptedProvider::new().with_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some("bg_run".into()),
            args_delta: json!({"action":"run","command":"sleep 0.3; echo background result"})
                .to_string(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]));
    // The first turn's answer, then the wake-up turn's. The third script is
    // what the wake-up consumes — nothing else may run.
    provider.script_text("started");
    provider.script_text("the background job finished");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, mut channels) = rt.block_on(async {
        SessionActor::spawn(SessionConfig {
            workspace_root: dir.path().to_path_buf(),
            paths: agent_paths::AppPaths::for_tests("kernel"),
            provider: provider.clone(),
            model: "test-model".into(),
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
            memory_enabled: false,
            memory_distill: false,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        })
    });
    let reg = actor.bg_registry();
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt {
        text: "start the build".into(),
    }))
    .unwrap();

    let prompts = rt.block_on(async {
        let mut prompts = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(ev) = channels.event_rx.recv().await {
                match ev {
                    agent_ipc::UiEvent::UserPrompt(text) => prompts.push(text),
                    agent_ipc::UiEvent::AssistantMessage(text)
                        if text.contains("background job finished") =>
                    {
                        break
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("the job's completion must wake the session");
        prompts
    });
    // Only the prompt the user typed — the wake-up invented none.
    assert_eq!(prompts, vec!["start the build".to_string()]);

    // The model's second request carried the completion notice.
    let seen = provider.seen_messages.lock().unwrap();
    // The tool round, the answer, then the wake-up — and nothing after it.
    assert_eq!(
        seen.len(),
        3,
        "one tool round, its answer, one wake-up: {seen:#?}"
    );
    let wake = &seen[2];
    // Job ids come from a process-wide counter that other tests in this
    // binary also draw from, so read this session's id instead of assuming.
    let id = reg.jobs()[0].id.clone();
    assert!(
        wake.iter()
            .any(|m| m.contains(&format!("background task {id} finished"))),
        "the wake-up must show the completion: {wake:#?}"
    );
}

/// A model that starts a new job on every wake-up cannot keep an unattended
/// session running: after `MAX_WAKES_IN_ROW` notice turns the next completion
/// waits for the user, and the user is told once.
#[test]
fn wake_ups_stop_after_the_cap() {
    use agent_llm::types::StreamChunk;
    use husk_host::command::UiCommand;
    use husk_host::session::{SessionActor, SessionConfig};

    let start_job = |call: &str| {
        vec![
            StreamChunk::ToolCallDelta {
                slot: 0,
                id: Some(call.into()),
                name: Some("bg_run".into()),
                args_delta: json!({"action":"run","command":"sleep 0.3; echo again"}).to_string(),
            },
            StreamChunk::Done {
                prompt_tokens: None,
                completion_tokens: None,
                cached_tokens: None,
            },
        ]
    };
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(agent_testkit::ScriptedProvider::new());
    // The prompt turn and three wake-ups, each starting one more job.
    for i in 0..4 {
        provider.push_script(start_job(&format!("c{i}")));
        provider.script_text("started another");
    }
    // What a fourth wake-up would consume — it must stay unused.
    provider.script_text("woke past the cap");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, mut channels) = rt.block_on(async {
        SessionActor::spawn(SessionConfig {
            workspace_root: dir.path().to_path_buf(),
            paths: agent_paths::AppPaths::for_tests("kernel"),
            provider: provider.clone(),
            model: "test-model".into(),
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
            memory_enabled: false,
            memory_distill: false,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        })
    });
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt {
        text: "keep building".into(),
    }))
    .unwrap();

    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), async {
            while let Some(ev) = channels.event_rx.recv().await {
                if let agent_ipc::UiEvent::SystemMessage(text) = &ev {
                    if text.contains("连续唤醒") {
                        break;
                    }
                }
            }
        })
        .await
        .expect("the cap must be announced");
        // Room for a wake-up that should not happen.
        tokio::time::sleep(Duration::from_millis(300)).await;
    });
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(
        seen.len(),
        8,
        "the prompt turn and three wake-ups, two requests each: {seen:#?}"
    );
}

/// A kill from the UI is news to the model: the idle session wakes on a
/// notice saying the user stopped the job, so the model never goes on
/// believing it still runs.
#[test]
fn a_kill_from_the_ui_wakes_the_model() {
    use agent_llm::types::StreamChunk;
    use husk_host::command::UiCommand;
    use husk_host::session::{SessionActor, SessionConfig};

    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(agent_testkit::ScriptedProvider::new().with_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some("bg_run".into()),
            args_delta: json!({"action":"run","command":"sleep 60"}).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]));
    provider.script_text("started");
    provider.script_text("noted, the user stopped it");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, mut channels) = rt.block_on(async {
        SessionActor::spawn(SessionConfig {
            workspace_root: dir.path().to_path_buf(),
            paths: agent_paths::AppPaths::for_tests("kernel"),
            provider: provider.clone(),
            model: "test-model".into(),
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
            memory_enabled: false,
            memory_distill: false,
            queued_prompts: Vec::new(),
            plugins: None,
            provider_name: "test".into(),
        })
    });
    let reg = actor.bg_registry();
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt {
        text: "start it".into(),
    }))
    .unwrap();

    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(20), async {
            // The first turn ends with the job still running.
            while let Some(ev) = channels.event_rx.recv().await {
                if matches!(&ev, agent_ipc::UiEvent::AssistantMessage(t) if t == "started") {
                    break;
                }
            }
            let id = reg.jobs()[0].id.clone();
            reg.kill(&id).await.unwrap();
            while let Some(ev) = channels.event_rx.recv().await {
                if matches!(&ev, agent_ipc::UiEvent::AssistantMessage(t) if t.contains("user stopped")) {
                    break;
                }
            }
        })
        .await
        .expect("the UI kill must wake the session");
    });

    let id = reg.jobs()[0].id.clone();
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen.len(), 3, "tool round, answer, one wake-up: {seen:#?}");
    assert!(
        seen[2]
            .iter()
            .any(|m| m.contains(&format!("background task {id} stopped by the user"))),
        "{:#?}",
        seen[2]
    );
}

/// The 后台 list survives a restart: a session spawned again under the same id
/// starts from the record its earlier process left, shows those entries as
/// restored, and appends its own jobs after them instead of overwriting the
/// record with only what ran since.
#[test]
fn a_respawned_session_keeps_the_earlier_jobs() {
    use agent_llm::types::StreamChunk;
    use husk_host::command::UiCommand;
    use husk_host::session::{SessionActor, SessionConfig, BG_JOBS};

    const ID: i64 = 4242;
    let dir = tempfile::tempdir().unwrap();
    let store = agent_store::SessionStore::open(dir.path()).unwrap();
    // What an earlier process wrote: one job it ran to the end, one it was
    // still running when the app closed.
    store
        .save_bg_jobs(
            ID,
            &json!([
                {"id": "bg-1", "command": "echo old", "started_at_ms": 11, "elapsed_ms": 5,
                 "notify": true, "has_own_sink": true,
                 "state": {"kind": "done", "label": "exited(0)", "code": 0, "reason": "exited"}},
                {"id": "bg-2", "command": "sleep 99", "started_at_ms": 12, "elapsed_ms": 5,
                 "notify": true, "has_own_sink": true, "state": {"kind": "running"}},
            ]),
        )
        .unwrap();

    let provider = Arc::new(agent_testkit::ScriptedProvider::new().with_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some("bg_run".into()),
            args_delta: json!({"action":"run","command":"echo fresh"}).to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]));
    provider.script_text("started");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut actor, mut channels) = rt.block_on(async {
        SessionActor::spawn_with_id(
            SessionConfig {
                workspace_root: dir.path().to_path_buf(),
                paths: agent_paths::AppPaths::for_tests("kernel"),
                provider: provider.clone(),
                model: "test-model".into(),
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
                memory_enabled: false,
                memory_distill: false,
                queued_prompts: Vec::new(),
                plugins: None,
                provider_name: "test".into(),
            },
            ID,
        )
    });
    // Read back before anything ran: the record, interrupted where it was cut.
    let restored = actor.bg_restored().lock().unwrap().clone();
    assert_eq!(restored.len(), 2, "{restored:?}");
    assert_eq!(restored[0]["id"], "bg-1@11");
    assert_eq!(restored[1]["state"]["label"], "应用已退出");

    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt { text: "go".into() }))
        .unwrap();

    let last = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(ev) = channels.event_rx.recv().await {
                if let agent_ipc::UiEvent::Custom { kind, payload } = ev {
                    let jobs = payload["jobs"].as_array().cloned().unwrap_or_default();
                    if kind == BG_JOBS && jobs.last().is_some_and(|j| j["state"]["kind"] == "done" && j.get("restored").is_none()) {
                        return payload;
                    }
                }
            }
            panic!("event stream ended");
        })
        .await
        .expect("a snapshot with the new job finished")
    });

    let jobs = last["jobs"].as_array().unwrap();
    let commands: Vec<&str> = jobs.iter().map(|j| j["command"].as_str().unwrap()).collect();
    assert_eq!(commands, ["echo old", "sleep 99", "echo fresh"], "{last}");
    // The new process numbers its jobs afresh; a restored row must not share
    // an id with one, or a removal would take the wrong row.
    assert!(jobs[2]["id"].as_str().unwrap().starts_with("bg-"));
    assert!(!jobs[2]["id"].as_str().unwrap().contains('@'));
    assert_ne!(jobs[0]["id"], jobs[2]["id"]);

    // The record on disk is the merged list, so the next restart keeps all three.
    let saved = agent_store::SessionStore::open(dir.path())
        .unwrap()
        .load_bg_jobs(ID)
        .unwrap();
    assert_eq!(saved.as_array().unwrap().len(), 3, "{saved}");
    // The log file dies with this process; the record keeps its tail, and the
    // live snapshot never carries it.
    assert!(
        saved[2]["log_tail"].as_str().unwrap_or_default().contains("fresh"),
        "{saved}"
    );
    assert!(jobs[2].get("log_tail").is_none(), "{last}");
}
