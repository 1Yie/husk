//! Metadata and batch guarantees for the tool set Husk actually assembles.

use std::sync::Arc;

use agent_kernel::tools::registry::ToolClass;
use agent_kernel::tools::ToolCtx;
use husk_host::session::base_registries;

#[test]
fn eligibility_matrix() {
    let reg = base_registries().0;
    for (name, observation) in [
        ("smart_read", true),
        ("list_dir", true),
        ("smart_grep", true),
        ("web_fetch", true),
        ("bash", false),
        ("apply_patch", false),
        ("fuzzy_patch", false),
        ("smart_test_runner", false),
        ("serena", false),
        ("skill", false),
        ("delegate", false),
        ("ask_question", false),
        ("todo", false),
        ("batch_execute", false),
    ] {
        let spec = reg.spec(name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(spec.class == ToolClass::Observation, observation, "{name}");
    }
}

#[test]
fn staged_flag_matches_the_registry() {
    let reg = base_registries().0;
    let schema = reg.request_schema();
    let specs: Vec<_> = schema
        .as_array()
        .unwrap()
        .iter()
        .map(|s| reg.spec(s["function"]["name"].as_str().unwrap()).unwrap())
        .collect();
    let mut staged: Vec<_> = specs.iter().filter(|s| s.staged).map(|s| s.name).collect();
    staged.sort();
    assert_eq!(staged, ["apply_patch", "fuzzy_patch"]);
    for name in staged {
        let spec = reg.spec(name).unwrap();
        assert!(!spec.readonly, "staged writer {name} must not be readonly");
        assert_eq!(spec.class, ToolClass::WorkspaceMutation);
    }
    let serena = reg.spec("serena").unwrap();
    assert_eq!(serena.class, ToolClass::WorkspaceMutation);
    assert!(!serena.staged, "serena must not write before approval");
}

#[test]
fn registry_carries_screenshot_and_computer() {
    let r = base_registries().0;
    assert!(r.spec("screenshot").is_some());
    assert!(r.spec("computer").is_some());
    assert!(r.is_readonly("screenshot"));
    assert!(!r.is_readonly("computer"));
}

#[test]
fn children_exclude_parent_capabilities() {
    let (_, child) = base_registries();
    for name in [
        "delegate",
        "remember",
        "computer",
        "ask_question",
        "goal_complete",
        "goal_blocked",
        "submit_plan",
    ] {
        assert!(child.spec(name).is_none(), "child must not hold {name}");
    }
    assert!(child.spec("smart_read").is_some());
    assert!(child.spec("bash").is_some());
}

#[test]
fn network_flag_only_on_fetch() {
    let reg = base_registries().0;
    for (name, want) in [
        ("web_fetch", true),
        ("smart_read", false),
        ("list_dir", false),
        ("smart_grep", false),
        ("batch_execute", false),
    ] {
        assert_eq!(reg.spec(name).unwrap().network, want, "{name}");
    }
}

/// Good items run, a shapeless item is rejected alone, and every section is reported.
#[tokio::test]
async fn a_real_batch_runs_and_isolates_a_shapeless_item() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    let reg = Arc::new(base_registries().0);
    let mut ctx = ToolCtx::new(dir.path());
    ctx.active_registry = std::sync::RwLock::new(Some(reg.clone()));
    let out = reg
        .dispatch(
            "batch_execute",
            serde_json::json!({"calls": [
                {"tool": "list_dir", "args": {"path": "."}},
                {"name": "smart_read", "arguments": {"path": "a.rs"}},
                {"path": "orphan.rs"}
            ]}),
            Arc::new(ctx),
        )
        .await
        .expect("a bad item must not fail the batch");
    let text = out.content;
    assert!(text.contains("list_dir (ok"), "{text}");
    assert!(text.contains("smart_read (ok"), "{text}");
    assert!(text.contains("rejected"), "{text}");
    assert!(text.contains("could not be parsed"), "{text}");
}
