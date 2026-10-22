use std::sync::Arc;

use agent_capabilities::{SandboxCtx, WorkspaceCtx};
use agent_ipc::UiEvent;
use agent_kernel::engine::{Engine, EngineIo};
use agent_kernel::permissions::{CallRisk, Decision, PermissionGate, PermissionRules};
use agent_kernel::tools::{ToolCtx, ToolRegistry};
use agent_llm::types::{ChatMessage, StreamChunk};
use agent_testkit::ScriptedProvider;

fn decision(gate: &PermissionGate, registry: &ToolRegistry, tool: &str, command: &str) -> Decision {
    let spec = registry.spec(tool).unwrap();
    let ctx = Arc::new(ToolCtx::new().with_workspace(std::env::temp_dir()));
    let review = spec.review.as_ref().unwrap()(&serde_json::json!({"command": command}), &ctx);
    gate.decide(tool, Some(spec.class), spec.readonly, review.risk, command)
}

#[test]
fn shell_tools_report_risk_to_each_permission_mode() {
    let registry = husk_host::session::base_registries().0;
    for tool in ["bash", "smart_test_runner"] {
        for mode in ["default", "acceptEdits", "dontAsk", "auto", "bypass"] {
            let gate = PermissionGate::from_mode_str(mode);
            for command in ["ls -la", "git status", "cat file.txt"] {
                assert_eq!(
                    decision(&gate, &registry, tool, command),
                    Decision::Allow,
                    "{tool} {mode} {command}"
                );
            }
            for command in ["cargo test", "mkdir demo", "cat a > b.txt"] {
                let d = decision(&gate, &registry, tool, command);
                match mode {
                    "auto" | "bypass" => assert_eq!(d, Decision::Allow),
                    "dontAsk" => assert!(matches!(d, Decision::Deny { .. })),
                    _ => assert!(matches!(d, Decision::Ask { .. })),
                }
            }
            for command in ["rm -rf target", "git reset --hard HEAD~1", "ls && rm -rf x"] {
                assert!(
                    matches!(
                        decision(&gate, &registry, tool, command),
                        Decision::Ask { .. }
                    ),
                    "{tool} {mode} {command}"
                );
            }
        }
    }
}

#[test]
fn rules_and_headless_mode_still_bound_shell_escalation() {
    let registry = husk_host::session::base_registries().0;
    let mut gate = PermissionGate::from_mode_str("bypass");
    gate.allow_tool_for_session("bash");
    assert_eq!(
        decision(&gate, &registry, "bash", "rm -rf target"),
        Decision::Allow
    );
    gate.merge_rules(PermissionRules {
        ask: ["bash".into()].into_iter().collect(),
        ..Default::default()
    });
    assert!(matches!(
        decision(&gate, &registry, "bash", "rm -rf target"),
        Decision::Ask { .. }
    ));
    gate.merge_rules(PermissionRules {
        deny: ["bash".into()].into_iter().collect(),
        ..Default::default()
    });
    assert!(matches!(
        decision(&gate, &registry, "bash", "ls"),
        Decision::Deny { .. }
    ));
    let headless = PermissionGate::for_subagent();
    assert!(matches!(
        decision(&headless, &registry, "bash", "rm -rf target"),
        Decision::Deny { .. }
    ));
}

/// An unknown tool's `command` argument grants no privilege by itself.
#[test]
fn unknown_tool_names_cannot_supply_readonly_or_mutation_privileges() {
    let gate = PermissionGate::from_mode_str("acceptEdits");
    for name in [
        "fuzzy_patch",
        "apply_patch",
        "write_file",
        "fs_patch",
        "remote_exec",
    ] {
        assert!(matches!(
            gate.decide(name, None, false, CallRisk::Normal, "ls"),
            Decision::Ask { .. }
        ));
    }
}

#[tokio::test]
async fn engine_uses_the_tools_review_instead_of_parsing_command_arguments() {
    for tool in ["bash", "smart_test_runner"] {
        // Both commands are harmless if a regression accidentally runs them.
        for (command, expected_cards) in [("echo inspected", 0), ("rm --help", 1)] {
            let dir = tempfile::tempdir().unwrap();
            let provider = Arc::new(ScriptedProvider::new());
            provider.push_script(vec![
                StreamChunk::ToolCallDelta {
                    slot: 0,
                    id: Some("risk-call".into()),
                    name: Some(tool.into()),
                    args_delta: serde_json::json!({"command": command}).to_string(),
                },
                StreamChunk::Done {
                    prompt_tokens: None,
                    completion_tokens: None,
                    cached_tokens: None,
                },
            ]);
            provider.script_text("done");
            let ctx = Arc::new(
                ToolCtx::new()
                    .with_workspace(dir.path())
                    .with_sandbox(Arc::new(agent_sandbox::NoneBackend)),
            );
            let mut engine = Engine::new(
                provider,
                Arc::new(husk_host::session::base_registries().0),
                ctx,
                "test",
                0.0,
            );
            let slot = engine.decision_slot();
            let (ui_tx, mut ui_rx) = agent_kernel::channels::UiSink::channel();
            let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel(4);
            let mut io = EngineIo {
                ui_tx,
                steer_rx,
                notice_rx: tokio::sync::mpsc::channel(4).1,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            let mut history = vec![ChatMessage::system("test")];
            let mut cards = 0;
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let turn = engine.run_turn(&mut io, &mut history, "go".into());
                tokio::pin!(turn);
                loop {
                    tokio::select! {
                        result = &mut turn => break result.unwrap(),
                        event = ui_rx.recv() => {
                            if let Some(UiEvent::ApprovalRequested { request_id, .. }) = event {
                                cards += 1;
                                *slot.lock().unwrap() = Some((
                                    request_id,
                                    agent_kernel::tools::registry::ToolVerdict {
                                        approved: false,
                                        action: None,
                                    },
                                ));
                            }
                        }
                    }
                }
            })
            .await
            .expect("turn must finish");
            assert_eq!(cards, expected_cards, "{tool} {command}");
            if expected_cards == 0 && tool == "bash" {
                assert!(
                    history.iter().any(|m| m
                        .content
                        .as_deref()
                        .is_some_and(|text| text.contains("inspected"))),
                    "{history:?}"
                );
            }
        }
    }
}
