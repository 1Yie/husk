use super::*;
use agent_ipc::AgentState;
use agent_kernel::tools::registry::AskChannel;
use agent_llm::types::StreamChunk;
use std::time::Duration;

use agent_testkit::ScriptedProvider;

#[tokio::test]
async fn headless_approval_and_denial_release_the_running_turn() {
    for yes in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let provider = Arc::new(ScriptedProvider::new());
        provider.push_script(vec![
            StreamChunk::ToolCallDelta {
                slot: 0, id: Some("write".into()), name: Some("apply_patch".into()),
                args_delta: serde_json::json!({"patch": "*** Begin Patch\n*** Add File: out.txt\n+test\n*** End Patch\n"}).to_string(),
            },
            StreamChunk::Done { prompt_tokens: None, completion_tokens: None, cached_tokens: None },
        ]);
        provider.script_text("done");
        let cfg = SessionConfig {
            workspace_root: dir.path().into(),
            paths: agent_paths::AppPaths::from_app_name("husk", ".husk").unwrap_or_else(|| agent_paths::AppPaths::new("", "", ".husk")),
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
            model_input: Vec::new(),
            compact_at: None,
            model_params: None,
            plugins: None,
            queued_prompts: Vec::new(),
            memory_enabled: false,
            memory_distill: false,
        };
        let (mut actor, mut channels) = SessionActor::spawn(cfg);
        let tx = actor.command_sender();
        let decision = actor.decision_writer();
        let ask = actor.ask_channel();
        let run = tokio::spawn(async move { actor.run().await });
        tx.send(UiCommand::Prompt {
            text: "test".into(),
        })
        .await
        .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let mut requested = false;
            while let Some(ev) = channels.event_rx.recv().await {
                requested |= matches!(ev, UiEvent::ApprovalRequested { .. });
                auto_decide(&ev, yes, &decision, &ask);
                match ev {
                    UiEvent::StateChanged(AgentState::Finished) => return requested,
                    UiEvent::StateChanged(AgentState::Failed(e)) => panic!("turn failed: {e}"),
                    _ => {}
                }
            }
            false
        })
        .await;
        run.abort();
        assert!(result.expect("approval was stuck behind the running turn"));
        assert_eq!(dir.path().join("out.txt").exists(), yes);
    }
}

#[tokio::test]
async fn headless_questions_get_first_option_or_empty_answer() {
    for yes in [false, true] {
        let (sink, mut rx) = agent_kernel::channels::UiSink::channel();
        let ask = Arc::new(AskChannel::new(Some(sink)));
        let requester = ask.clone();
        let run = tokio::spawn(async move {
            requester
                .ask(
                    "choose".into(),
                    vec![agent_ipc::events::AskOption {
                        label: "first".into(),
                        description: Some("test".into()),
                    }],
                    None,
                )
                .await
                .unwrap()
        });
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(ev, UiEvent::QuestionAsked { .. }));
        auto_decide(&ev, yes, &std::sync::Mutex::new(None), &ask);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), run)
                .await
                .unwrap()
                .unwrap(),
            if yes { "first" } else { "" }
        );
    }
}
