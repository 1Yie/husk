use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agent_ipc::UiEvent;
use agent_llm::{BoxStream, ChatMessage, LlmProvider, ModelParams, StreamChunk};
use husk_host::command::UiCommand;
use husk_host::session::{SessionActor, SessionConfig};

const TASK: &str = "INDEPENDENT-GATED-REVIEW";
const QUESTION: &str = "REVIEW-NEEDS-CLARIFICATION";
const REPORT: &str = "INDEPENDENT-REVIEW-FINDINGS";
const SUMMARY: &str = "PARENT-ACCEPTED-REVIEW";

fn call(stage: usize, name: &str, args: serde_json::Value) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ToolCallDelta {
            slot: 0,
            id: Some(format!("call-{stage}")),
            name: Some(name.into()),
            args_delta: args.to_string(),
        },
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]
}

fn text(body: &str) -> Vec<StreamChunk> {
    vec![
        StreamChunk::ContentDelta(body.into()),
        StreamChunk::Done {
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
        },
    ]
}

struct ReviewProvider {
    mode: &'static str,
    question: bool,
    parent_calls: AtomicUsize,
    child_calls: AtomicUsize,
    gate: tokio::sync::Notify,
    child_entered: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl LlmProvider for ReviewProvider {
    fn id(&self) -> &'static str {
        "gated-review"
    }

    async fn chat_stream(
        &self,
        _model: &str,
        messages: &[ChatMessage],
        tools: Option<serde_json::Value>,
        _temperature: f32,
        _reasoning_effort: Option<&str>,
        _params: &ModelParams,
    ) -> anyhow::Result<BoxStream<StreamChunk>> {
        let bodies: Vec<String> = messages.iter().filter_map(|m| m.content.clone()).collect();
        let chunks = if bodies.iter().any(|body| body == TASK) {
            let schema = tools.as_ref().unwrap().as_array().unwrap();
            for banned in ["delegate", "answer_child", "wait_child"] {
                assert!(
                    !schema.iter().any(|tool| tool["function"]["name"] == banned),
                    "a child must not receive {banned}"
                );
            }
            let stage = self.child_calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.child_entered.notify_one();
            self.gate.notified().await;
            if self.question && stage == 1 {
                call(
                    stage,
                    "ask_parent",
                    serde_json::json!({"question": QUESTION}),
                )
            } else {
                assert_eq!(stage, if self.question { 2 } else { 1 });
                text(REPORT)
            }
        } else {
            let stage = self.parent_calls.fetch_add(1, Ordering::SeqCst) + 1;
            match stage {
                1 => call(
                    stage,
                    "delegate",
                    serde_json::json!({"task": TASK, "readonly": true, "background": true}),
                ),
                2 => text("The independent review is scheduled."),
                3 | 5 if stage == 3 || self.question => {
                    let id = bodies
                        .iter()
                        .find_map(|body| {
                            body.split("child_id ")
                                .nth(1)?
                                .split(')')
                                .next()?
                                .parse::<u64>()
                                .ok()
                        })
                        .expect("the parent has the real delegated child id");
                    call(stage, "wait_child", serde_json::json!({"child_id": id}))
                }
                4 if self.question => {
                    assert_eq!(
                        bodies.iter().filter(|body| body.contains(QUESTION)).count(),
                        1
                    );
                    let id = bodies
                        .iter()
                        .find_map(|body| {
                            body.split("child_id ")
                                .nth(1)?
                                .split(')')
                                .next()?
                                .parse::<u64>()
                                .ok()
                        })
                        .unwrap();
                    call(
                        stage,
                        "answer_child",
                        serde_json::json!({
                            "child_id": id, "answer": "Finish the review.", "background": true
                        }),
                    )
                }
                4 | 6 if stage == if self.question { 6 } else { 4 } => {
                    assert_eq!(
                        bodies.iter().filter(|body| body.contains(REPORT)).count(),
                        1
                    );
                    if self.mode == "goal" {
                        call(
                            stage,
                            "goal_complete",
                            serde_json::json!({"summary": SUMMARY}),
                        )
                    } else if self.mode == "plan" {
                        call(
                            stage,
                            "submit_plan",
                            serde_json::json!({
                                "summary": SUMMARY,
                                "steps": [{
                                    "title": "Use the delegated findings",
                                    "detail": "Apply the accepted review.",
                                    "files": []
                                }],
                                "verification": ["The independent review is complete."]
                            }),
                        )
                    } else {
                        text(SUMMARY)
                    }
                }
                5 if self.mode == "plan" || (self.mode == "goal" && !self.question) => {
                    text(SUMMARY)
                }
                7 if self.mode == "goal" && self.question => text(SUMMARY),
                _ => anyhow::bail!("unexpected parent sampling round {stage}"),
            }
        };
        Ok(Box::pin(futures::stream::iter(chunks.into_iter().map(Ok))))
    }
}

#[test]
fn host_wait_is_parent_only_and_available_to_readonly_modes() {
    let (parent, child) = husk_host::session::base_registries();
    assert!(
        parent
            .spec("wait_child")
            .expect("parent wait tool")
            .readonly
    );
    assert!(parent.readonly_only().spec("wait_child").is_some());
    assert!(child.spec("wait_child").is_none());
}

async fn waiting_parent(mode: &'static str, question: bool) {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(ReviewProvider {
        mode,
        question,
        parent_calls: AtomicUsize::new(0),
        child_calls: AtomicUsize::new(0),
        gate: tokio::sync::Notify::new(),
        child_entered: tokio::sync::Notify::new(),
    });
    let (mut actor, mut channels) = SessionActor::spawn(SessionConfig {
        workspace_root: dir.path().to_path_buf(),
        paths: agent_paths::AppPaths::for_tests("delegate-wait"),
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
        approval_timeout: None,
        question_timeout: None,
        model_input: Vec::new(),
        model_params: None,
        memory_enabled: false,
        memory_distill: false,
        queued_prompts: Vec::new(),
        plugins: None,
        provider_name: "test".into(),
    });
    actor
        .handle(UiCommand::Prompt {
            text: "Start an independent review for later.".into(),
        })
        .await;
    actor
        .handle(UiCommand::SetAgentMode { mode: mode.into() })
        .await;
    let mut running = tokio::spawn(async move {
        actor
            .handle(UiCommand::Prompt {
                text: "Accept the delegated findings when ready.".into(),
            })
            .await;
        actor
    });
    let mut parent_tools = Vec::new();
    for wait_index in 0..if question { 2 } else { 1 } {
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = channels.event_rx.recv().await {
                if let UiEvent::ToolCallStarted { name, .. } = event {
                    let waiting = name == "wait_child";
                    parent_tools.push(name);
                    if waiting {
                        return;
                    }
                }
            }
            panic!("the parent never started its wait");
        })
        .await
        .expect("the parent must reach the wait");
        tokio::time::timeout(Duration::from_secs(3), provider.child_entered.notified())
            .await
            .expect("the child reaches its model gate");
        assert!(
            tokio::time::timeout(Duration::from_millis(40), &mut running)
                .await
                .is_err(),
            "{mode}: the parent stays in the same turn while waiting"
        );
        assert_eq!(
            provider.parent_calls.load(Ordering::SeqCst),
            3 + wait_index * 2
        );
        assert!(
            parent_tools.iter().all(|name| matches!(
                name.as_str(),
                "delegate" | "subagent" | "wait_child" | "answer_child" | "ask_parent"
            )),
            "waiting must not issue file reads: {parent_tools:?}"
        );
        provider.gate.notify_one();
    }
    let actor = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the parent resumes after the child handoff")
        .unwrap();
    assert_eq!(
        provider.parent_calls.load(Ordering::SeqCst),
        if question {
            7
        } else if mode != "build" {
            5
        } else {
            4
        }
    );
    assert_eq!(
        provider.child_calls.load(Ordering::SeqCst),
        if question { 2 } else { 1 }
    );
    assert_eq!(
        actor
            .history()
            .iter()
            .filter_map(|message| message.content.as_deref())
            .filter(|body| body.contains("目标尚未宣告完成"))
            .count(),
        0,
        "a live wait must not produce goal nudges"
    );
    if mode == "goal" {
        let mut completed = false;
        while let Some(event) = channels.event_rx.try_recv() {
            if let UiEvent::ToolCallFinished { name, ok, .. } = event {
                if name == "goal_complete" {
                    assert!(ok, "the original goal contract stays active");
                    completed = true;
                }
            }
        }
        assert!(completed, "goal completion follows the accepted report");
    }
}

#[tokio::test]
async fn build_parent_waits_and_resumes_without_repeating_the_review() {
    waiting_parent("build", false).await;
}

#[tokio::test]
async fn plan_parent_waits_and_resumes_without_repeating_the_review() {
    waiting_parent("plan", false).await;
}

#[tokio::test]
async fn goal_parent_waits_and_completes_the_same_goal_contract() {
    waiting_parent("goal", false).await;
}

#[tokio::test]
async fn goal_parent_answers_a_child_question_and_waits_for_its_final_report() {
    waiting_parent("goal", true).await;
}
