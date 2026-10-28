//! Observation routing through the registry Husk actually assembles: what the
//! model is told about each tool, and what it is told after each call.
//!
//! These pin the harness side — descriptions, notes, ledger lifetime. Whether
//! a given model follows them is a question for a model evaluation, not for
//! this file.

use std::sync::Arc;

use agent_capabilities::{SandboxCtx, WorkspaceCtx};
use agent_coding::observe::ObservationLedger;
use agent_kernel::tools::registry::ToolCtx;
use agent_kernel::tools::ToolRegistry;
use husk_host::session::{base_registries, SYSTEM_PROMPT_TEMPLATE};

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let body: String = (1..=30).map(|i| format!("fn f{i}() {{}}\n")).collect();
    std::fs::write(dir.path().join("src/lib.rs"), body).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() { needle(); }\n").unwrap();
    dir
}

/// The session's root context as far as observation goes: workspace, the
/// registry for batches, a ledger, and the detected sandbox for `bash`.
fn session(root: &std::path::Path) -> (Arc<ToolRegistry>, Arc<ToolCtx>, Arc<ObservationLedger>) {
    let reg = Arc::new(base_registries().0);
    let mut ctx = ToolCtx::new().with_workspace(root);
    ctx.active_registry = std::sync::RwLock::new(Some(reg.clone()));
    let ledger = ObservationLedger::install(&mut ctx);
    let (detected, _loud) = agent_sandbox::detect_backend();
    ctx.install_sandbox(Arc::from(detected));
    (reg, Arc::new(ctx), ledger)
}

async fn call(
    reg: &ToolRegistry,
    ctx: &Arc<ToolCtx>,
    tool: &str,
    args: serde_json::Value,
) -> String {
    match reg.dispatch(tool, args, ctx.clone()).await {
        Ok(out) => out.content,
        Err(e) => format!("ERR {e}"),
    }
}

fn description(reg: &ToolRegistry, tool: &str) -> String {
    reg.spec(tool).unwrap().schema["description"]
        .as_str()
        .unwrap()
        .to_string()
}

const IDENTICAL: &str = "identical to the output this same call returned earlier";
const ROUTING: &str = "[routing: this command only looked at workspace files";

/// Reading, searching and listing are claimed by the file tools; the shell is
/// described as the tool for running things, and says so about plain reads.
#[test]
fn descriptions_split_observing_from_running() {
    let reg = base_registries().0;
    let read = description(&reg, "smart_read");
    assert!(read.contains("in place of `cat` / `sed -n`"), "{read}");
    assert!(read.contains("batch_execute"), "{read}");
    let grep = description(&reg, "smart_grep");
    assert!(grep.contains("instead of `grep` / `rg`"), "{grep}");
    assert!(grep.contains("`context: N`"), "{grep}");
    assert!(description(&reg, "list_dir").contains("in place of `ls`"));
    let bash = description(&reg, "bash");
    for duty in ["builds", "tests", "git", "diagnostics"] {
        assert!(bash.contains(duty), "bash must still own {duty}: {bash}");
    }
    assert!(
        bash.contains("Not for looking at workspace files"),
        "{bash}"
    );
    // The schema still advertises every argument the tool accepts.
    let params = &reg.spec("bash").unwrap().schema["parameters"]["properties"];
    for arg in ["command", "timeout_secs", "persistent"] {
        assert!(params.get(arg).is_some(), "bash lost `{arg}`");
    }
    // `context` is the tool's own argument now, not something the host adds.
    let params = &reg.spec("smart_grep").unwrap().schema["parameters"]["properties"];
    for arg in ["pattern", "path", "glob", "max_hits", "context"] {
        assert!(params.get(arg).is_some(), "smart_grep lacks `{arg}`");
    }
}

/// The prompt the model actually receives carries the routing and reuse rules.
#[test]
fn the_prompt_separates_observation_from_execution_and_asks_for_reuse() {
    let p = SYSTEM_PROMPT_TEMPLATE;
    assert!(p.contains("Looking at the workspace is not running a command"));
    assert!(p.contains("Reuse what you already have"));
    assert!(p.contains("past a `[truncated …]` marker"));
    assert!(p.contains("`bash` is a fallback for what the file tools cannot do, not a retry"));
}

#[test]
fn the_prompt_prefers_semantic_navigation_without_changing_tool_boundaries() {
    let p = SYSTEM_PROMPT_TEMPLATE;
    for guidance in [
        "call `blueprint` locate **before reading code**",
        "when `serena` is among your available tools and the call is permitted",
        "`get_symbols_overview` for a named source file",
        "`find_symbol` for a named function/class/method",
        "Before behavior changes or renames, inspect `find_referencing_symbols`",
        "`tool: \"serena_list_tools\"`",
        "`arguments: {\"name\": \"<tool>\"}`",
        "a narrow `smart_read` range",
        "Use smart tools for docs/configs, exact text checks and linked repositories",
        "Serena currently indexes only the main workspace root, not linked roots",
        "unavailable, permission-restricted, unsupported for the language, or a call fails",
        "fall back reasonably to `smart_read` (`outline` for structure) / `smart_grep`",
        "do not block progress, bypass restrictions, or repeat failing calls",
        "`serena` remains mutation-class and cannot be batched, even for navigation or catalog discovery",
        "A plan-mode registry does not carry them at all",
        "2+, independent         → batch_execute — ALWAYS, one call",
        "**Do not batch dependent operations.**",
        "absolute paths only",
    ] {
        assert!(p.contains(guidance), "missing {guidance}");
    }
    // The routing table and the decision tree must agree with the prose.
    assert!(p.contains("| `serena` | When available and permitted:"));
    assert!(p.contains("Main-root semantic navigation, serena available + permitted"));
    assert!(p.contains("linked roots or semantic fallback"));
    assert!(!p.contains("| `smart_grep`, `list_dir` | Locating code:"));
}

/// `grep -C` without the shell: hits come back with their surroundings.
#[tokio::test]
async fn grep_context_returns_surrounding_lines() {
    let ws = workspace();
    let (reg, ctx, _) = session(ws.path());
    let out = call(
        &reg,
        &ctx,
        "smart_grep",
        serde_json::json!({"pattern": "fn f10\\(", "context": 2}),
    )
    .await;
    for want in [
        "src/lib.rs-8 │ fn f8()",
        "src/lib.rs:10 │ fn f10()",
        "src/lib.rs-12 │ fn f12()",
    ] {
        assert!(out.contains(want), "missing `{want}`:\n{out}");
    }
    assert!(out.contains("── 1 matches"), "summary kept:\n{out}");
    // Without `context` the output is the search's own.
    let plain = call(
        &reg,
        &ctx,
        "smart_grep",
        serde_json::json!({"pattern": "fn f10\\("}),
    )
    .await;
    assert!(plain.starts_with("src/lib.rs:10 │ fn f10()"), "{plain}");
    assert!(!plain.contains("src/lib.rs-9"), "{plain}");
}

/// Two independent observations in one batch, each noted by the shared
/// ledger: a direct repeat afterwards is recognised.
#[tokio::test]
async fn a_batch_shares_the_ledger_with_direct_calls() {
    let ws = workspace();
    let (reg, ctx, _) = session(ws.path());
    let out = call(
        &reg,
        &ctx,
        "batch_execute",
        serde_json::json!({"calls": [
            {"tool": "smart_read", "args": {"path": "src/main.rs"}},
            {"tool": "smart_grep", "args": {"pattern": "needle"}}
        ]}),
    )
    .await;
    assert!(out.contains("2 calls · 2 ok"), "{out}");
    let again = call(
        &reg,
        &ctx,
        "smart_read",
        serde_json::json!({"path": "src/main.rs"}),
    )
    .await;
    assert!(again.contains(IDENTICAL), "{again}");
    assert!(
        again.contains("fn main() { needle(); }"),
        "the full result still comes back:\n{again}"
    );
}

/// The same read twice is noted; a different range, or a continuation past a
/// truncation, is new information and is not.
#[tokio::test]
async fn only_a_true_repeat_is_noted() {
    let ws = workspace();
    let (reg, ctx, _) = session(ws.path());
    let first = call(
        &reg,
        &ctx,
        "smart_read",
        serde_json::json!({"path": "src/lib.rs", "start": 1, "end": 5}),
    )
    .await;
    assert!(!first.contains("[note"), "{first}");
    // Key order does not make it a different call.
    let repeat = call(
        &reg,
        &ctx,
        "smart_read",
        serde_json::json!({"end": 5, "start": 1, "path": "src/lib.rs"}),
    )
    .await;
    assert!(repeat.contains(IDENTICAL), "{repeat}");
    let next = call(
        &reg,
        &ctx,
        "smart_read",
        serde_json::json!({"path": "src/lib.rs", "start": 6, "end": 10}),
    )
    .await;
    assert!(
        !next.contains("[note"),
        "a new range is not a repeat:\n{next}"
    );
}

/// After an edit the re-read says the file moved, and is not called a repeat.
#[tokio::test]
async fn a_changed_file_is_flagged_on_the_next_read() {
    let ws = workspace();
    let (reg, ctx, _) = session(ws.path());
    let args = serde_json::json!({"path": "src/main.rs"});
    call(&reg, &ctx, "smart_read", args.clone()).await;
    std::fs::write(ws.path().join("src/main.rs"), "fn main() { other(); }\n").unwrap();
    let out = call(&reg, &ctx, "smart_read", args).await;
    assert!(out.contains("other()"), "{out}");
    assert!(out.contains("changed since your previous read"), "{out}");
    assert!(!out.contains(IDENTICAL), "{out}");
}

/// The ledger forgets with the history: after a rewind nothing is "seen".
#[tokio::test]
async fn a_cleared_ledger_notes_nothing() {
    let ws = workspace();
    let (reg, ctx, ledger) = session(ws.path());
    let args = serde_json::json!({"path": "src/main.rs"});
    call(&reg, &ctx, "smart_read", args.clone()).await;
    ledger.clear();
    let out = call(&reg, &ctx, "smart_read", args).await;
    assert!(!out.contains("[note"), "{out}");
}

/// A failure that repeats unchanged says so. One outside the workspace keeps
/// its structured refusal — the engine, not a note, turns that into the grant
/// card, so flattening it into prose here would hide the question.
#[tokio::test]
async fn failures_point_to_a_change_not_a_retry() {
    let ws = workspace();
    let (reg, ctx, _) = session(ws.path());
    let missing = serde_json::json!({"path": "src/nope.rs"});
    let first = call(&reg, &ctx, "smart_read", missing.clone()).await;
    assert!(first.starts_with("ERR"), "{first}");
    assert!(!first.contains("already failed"), "{first}");
    let second = call(&reg, &ctx, "smart_read", missing).await;
    assert!(
        second.contains("already failed with this exact error"),
        "{second}"
    );

    let outside = call(
        &reg,
        &ctx,
        "smart_read",
        serde_json::json!({"path": "/etc/hostname"}),
    )
    .await;
    assert!(outside.contains("path escapes workspace"), "{outside}");
    // No note: `OutOfScope` travels intact so the engine can ask for a grant.
    assert!(!outside.contains("[note"), "{outside}");
}

/// Running things through `bash` gets no remark; a plain workspace read does,
/// and when it re-reads an unchanged file the note says so.
#[cfg(unix)]
#[tokio::test]
async fn bash_runs_everything_and_only_annotates_plain_observation() {
    let ws = workspace();
    let (reg, ctx, _) = session(ws.path());
    for cmd in ["echo built && true", "git --version || true", "ls /"] {
        let out = call(&reg, &ctx, "bash", serde_json::json!({"command": cmd})).await;
        assert!(out.contains("── exit"), "{cmd} must run:\n{out}");
        assert!(
            !out.contains("[routing"),
            "{cmd} is not workspace observation:\n{out}"
        );
    }

    let out = call(
        &reg,
        &ctx,
        "bash",
        serde_json::json!({"command": "grep -rn needle src"}),
    )
    .await;
    assert!(
        out.contains("src/main.rs:1:"),
        "the command's output comes back:\n{out}"
    );
    assert!(out.contains(ROUTING) && out.contains("smart_grep"), "{out}");

    call(
        &reg,
        &ctx,
        "smart_read",
        serde_json::json!({"path": "src/lib.rs"}),
    )
    .await;
    let out = call(
        &reg,
        &ctx,
        "bash",
        serde_json::json!({"command": "sed -n '1,3p' src/lib.rs"}),
    )
    .await;
    assert!(out.contains("fn f1()"), "{out}");
    assert!(out.contains("is unchanged since your smart_read"), "{out}");

    let again = call(
        &reg,
        &ctx,
        "bash",
        serde_json::json!({"command": "sed -n '1,3p' src/lib.rs"}),
    )
    .await;
    assert!(
        again.contains("identical to the output this same command returned"),
        "{again}"
    );
}
