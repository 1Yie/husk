//! Interrupt must reach a running `bash`, not just the gap between tool calls.
//! The user hits stop during a long build; the process tree dies with the
//! dropped dispatch and the turn settles in well under a second.
#![cfg(unix)]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use agent_llm::types::StreamChunk;
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

/// Whether any process in the host's view still carries `marker` in its argv —
/// the bwrap supervisor, the shell, or `sleep`. bwrap runs `--unshare-pid`
/// (its own view of pids) but the host still lists every process.
fn process_with_marker(marker: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        if String::from_utf8_lossy(&cmdline).contains(marker) {
            return true;
        }
    }
    false
}

#[test]
fn cancel_kills_a_running_bash() {
    let dir = tempfile::tempdir().unwrap();
    // A marker only this test's process tree carries.
    let marker = format!("husk-cancel-marker-{}", std::process::id());
    let provider = Arc::new(agent_testkit::ScriptedProvider::new().with_script(vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some("c1".into()),
            name: Some("bash".into()),
            // Long enough that only a cancel can end it inside the test.
            args_delta: serde_json::json!({
                "command": format!("sleep 30 # {marker}"),
                "timeout_secs": 60
            })
            .to_string(),
        },
        StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
    ]));
    // A second request would mean the turn carried on after the cancel.
    provider.script_text("carried on");

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
    let cancel = actor.cancel_writer();
    let cmd = actor.command_sender();
    rt.spawn(async move { actor.run().await });
    rt.block_on(cmd.send(UiCommand::Prompt {
        text: "run the build".into(),
    }))
    .unwrap();

    // Same path the UI takes: the flag is written directly, bypassing the pump.
    let stopper = cancel.clone();
    rt.spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        stopper.store(true, Ordering::Relaxed);
    });

    let started = std::time::Instant::now();
    let outcome = rt.block_on(async {
        let mut cancelled = false;
        let mut finished_ok = None;
        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(ev) = channels.event_rx.recv().await {
                match ev {
                    agent_ipc::UiEvent::SystemMessage(text) if text.contains("已被用户中断") =>
                    {
                        cancelled = true;
                    }
                    agent_ipc::UiEvent::ToolCallFinished { name, ok, .. } if name == "bash" => {
                        finished_ok = Some(ok);
                    }
                    _ => {}
                }
                if cancelled && finished_ok == Some(false) {
                    break;
                }
            }
        })
        .await
        .expect("the cancel must land while bash runs");
        (cancelled, finished_ok)
    });
    let elapsed = started.elapsed();

    assert!(outcome.0, "the turn announced the cancel");
    assert_eq!(outcome.1, Some(false), "the running card settled as failed");
    assert!(
        elapsed < Duration::from_secs(5),
        "cancel waited for the command: {elapsed:?}"
    );
    // One request: nothing was sent to the model after the interruption.
    assert_eq!(provider.calls.lock().unwrap().len(), 1);

    // And the command itself is gone — not merely abandoned while `sleep`
    // keeps burning its timeout in the background.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while process_with_marker(&marker) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !process_with_marker(&marker),
        "the process tree survived the cancel"
    );
}
