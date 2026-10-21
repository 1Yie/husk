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
