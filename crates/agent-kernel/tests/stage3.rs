//! Stage-3 acceptance tests: registry dispatch, builtin tools, path-escape
//! guard, patch pipeline — all in-process, no network.

use std::sync::Arc;

use agent_kernel::tools::{ToolCtx, ToolRegistry};

fn registry() -> ToolRegistry {
    ToolRegistry::with_builtins()
}

fn ctx_at(dir: &std::path::Path) -> Arc<ToolCtx> {
    Arc::new(ToolCtx::new(dir))
}

#[tokio::test]
async fn registry_lists_builtins() {
    let r = registry();
    assert_eq!(r.len(), 11);
    let schema = r.request_schema();
    let names: Vec<&str> = schema
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    for expected in [
        "smart_read",
        "fuzzy_patch",
        "apply_patch",
        "smart_test_runner",
        "list_dir",
        "smart_grep",
        "bash",
        "todo",
        "serena",
        "web_fetch",
        "webfetch",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
    // readonly flags per permission contract
    assert!(r.is_readonly("smart_read"));
    assert!(r.is_readonly("smart_grep"));
    assert!(r.is_readonly("list_dir"));
    assert!(r.is_readonly("web_fetch"));
    assert!(r.is_readonly("webfetch"));
    assert!(!r.is_readonly("fuzzy_patch"));
    assert!(!r.is_readonly("bash"));
}

#[tokio::test]
async fn unknown_tool_teaches() {
    let dir = tempfile::tempdir().unwrap();
    let err = registry()
        .dispatch("nonexistent", serde_json::json!({}), ctx_at(dir.path()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"));
    assert!(err.to_string().contains("smart_read")); // lists what's available
}

#[tokio::test]
async fn path_escape_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let err = registry()
        .dispatch(
            "smart_read",
            serde_json::json!({"path": "../../etc/passwd", "mode": "range"}),
            ctx_at(dir.path()),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("escapes workspace"));
}

#[tokio::test]
async fn smart_read_range_and_search() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn one() {}\nfn two() {\n    x();\n}\n").unwrap();

    let r = registry();
    // range mode: numbered lines + content_hash
    let res = r
        .dispatch("smart_read", serde_json::json!({"path": "a.rs", "mode": "range"}), ctx_at(dir.path()))
        .await
        .unwrap();
    assert!(res.content.contains("1 │ fn one() {}"));
    assert!(res.content.contains("content_hash"));

    // outline mode: signature skeleton, bodies folded
    let res = r
        .dispatch("smart_read", serde_json::json!({"path": "a.rs", "mode": "outline"}), ctx_at(dir.path()))
        .await
        .unwrap();
    assert!(res.content.contains("fn one()"));
    assert!(res.content.contains("fn two()"));
    assert!(!res.content.contains("    x();")); // body folded

    // search mode
    let res = r
        .dispatch("smart_read", serde_json::json!({"path": "a.rs", "mode": "search", "pattern": "two"}), ctx_at(dir.path()))
        .await
        .unwrap();
    assert!(res.content.contains("2 │ fn two()"));
}

#[tokio::test]
async fn fuzzy_patch_full_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.rs");
    std::fs::write(&file, "pub fn run() {\n    step_one();\n}\n").unwrap();

    let r = registry();
    let res = r
        .dispatch(
            "fuzzy_patch",
            serde_json::json!({
                "path": "lib.rs",
                "search": "    step_one();",
                "replace": "    step_two();\n    step_one();",
            }),
            ctx_at(dir.path()),
        )
        .await
        .unwrap();
    assert_eq!(res.ui_type, Some("diff"));
    assert!(res.content.contains("patched lib.rs"));
    assert!(res.content.contains("step_two"));

    // P1-c: the tool now returns a `PendingWrite` instead of writing — the
    // ENGINE commits it post-approval. Emulate that commit here.
    let pw = res.pending_write.into_iter().next()
        .expect("fuzzy_patch returns a PendingWrite");
    std::fs::write(&pw.path, &pw.content).unwrap();

    let after = std::fs::read_to_string(&file).unwrap();
    assert!(after.contains("step_two();\n    step_one();"));
}

#[tokio::test]
async fn fuzzy_patch_hash_guard() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();

    let r = registry();
    // get real hash
    let read = r
        .dispatch("smart_read", serde_json::json!({"path": "a.rs", "mode": "range"}), ctx_at(dir.path()))
        .await
        .unwrap();
    let hash = read
        .content
        .split("content_hash: ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
        .to_string();

    // drift the file, then patch with the stale hash → refused
    std::fs::write(dir.path().join("a.rs"), "fn a() { changed(); }\n").unwrap();
    let err = r
        .dispatch(
            "fuzzy_patch",
            serde_json::json!({
                "path": "a.rs",
                "search": "fn a() {}",
                "replace": "fn a() { patched(); }",
                "expected_hash": hash,
            }),
            ctx_at(dir.path()),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("drifted"));
}

#[tokio::test]
async fn list_dir_bounded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
    std::fs::write(dir.path().join(".hidden"), "").unwrap();

    let res = registry()
        .dispatch("list_dir", serde_json::json!({"path": "."}), ctx_at(dir.path()))
        .await
        .unwrap();
    assert!(res.content.contains("src/"));
    assert!(res.content.contains("Cargo.toml"));
    assert!(!res.content.contains(".hidden"));
}

#[tokio::test]
async fn smart_grep_finds_and_counts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn target() {}\nfn other() {}\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "target();\n").unwrap();

    let res = registry()
        .dispatch(
            "smart_grep",
            serde_json::json!({"pattern": "target", "glob": "*.rs"}),
            ctx_at(dir.path()),
        )
        .await
        .unwrap();
    assert!(res.content.contains("a.rs:1"));
    assert!(res.content.contains("b.rs:1"));
    assert!(res.content.contains("2 match"));
}

#[tokio::test]
async fn bash_truncates_and_reports_exit() {
    let dir = tempfile::tempdir().unwrap();
    let r = registry();

    let res = r
        .dispatch("bash", serde_json::json!({"command": "echo hi && echo huge && seq 1 5000"}), ctx_at(dir.path()))
        .await
        .unwrap();
    assert!(res.content.contains("exit 0"));
    assert!(res.content.contains("truncated")); // 5000 lines > 20KB → folded

    let res = r
        .dispatch("bash", serde_json::json!({"command": "exit 3"}), ctx_at(dir.path()))
        .await
        .unwrap();
    assert!(res.content.contains("exit 3"));
}

#[tokio::test]
async fn test_runner_distills_failures() {
    let dir = tempfile::tempdir().unwrap();
    // Emit a fake cargo-test failure log: pass noise + one failure block.
    let script = r#"echo 'test a ... ok'; echo 'test b ... FAILED'; echo 'assertion failed: left == right'; echo 'test result: FAILED. 1 failed'"#;
    let res = registry()
        .dispatch(
            "smart_test_runner",
            serde_json::json!({"command": script}),
            ctx_at(dir.path()),
        )
        .await
        .unwrap();
    assert!(res.content.contains("test b ... FAILED"));
    assert!(res.content.contains("assertion"));
    // Pass-noise line must not survive distillation — the header echoes the
    // command, so assert on the distilled body after the first line.
    let body = res.content.lines().skip(1).collect::<Vec<_>>().join("\n");
    assert!(!body.contains("test a ... ok"));
}
