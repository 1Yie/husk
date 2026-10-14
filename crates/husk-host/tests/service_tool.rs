//! End-to-end: bash(persistent) → service registry → logs/stop,
//! through the real sandbox backend (bwrap on Linux).
#![cfg(unix)]

use std::sync::Arc;

use husk_host::sandbox_prefs::{SandboxLimits, SwitchableSandbox};
use agent_capabilities::{SandboxCtx, WorkspaceCtx};
use agent_kernel::tools::registry::ToolCtx;

/// A context with the detected backend installed, as a shell-capable host has.
fn ctx_with_sandbox(root: &std::path::Path) -> ToolCtx {
    let (detected, _loud) = agent_sandbox::detect_backend();
    let mut ctx = ToolCtx::new().with_workspace(root);
    ctx.install_sandbox(Arc::from(detected));
    ctx
}

fn call(spec_name: &str, args: serde_json::Value, ctx: Arc<ToolCtx>) -> String {
    let spec = match spec_name {
        "bash" => agent_tools_shell::bash::spec(),
        "service" => agent_tools_shell::services::spec(),
        _ => unreachable!(),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on((spec.exec)(args, ctx))
        .map(|r| r.content)
        .unwrap_or_else(|e| format!("ERR {e:?}"))
}

#[test]
fn persistent_service_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Arc::new(ctx_with_sandbox(dir.path()));

    // spawn: a loop that prints immediately then idles.
    let out = call(
        "bash",
        serde_json::json!({
            "command": "for i in 1 2 3; do echo tick$i; sleep 0.2; done; echo ready; sleep 60",
            "persistent": true
        }),
        ctx.clone(),
    );
    assert!(out.contains("svc-"), "spawn should return a service id: {out}");
    let id = out.split_whitespace()
        .find(|t| t.starts_with("id="))
        .unwrap()
        .trim_start_matches("id=")
        .to_string();

    std::thread::sleep(std::time::Duration::from_millis(800));

    // list shows it running.
    let out = call("service", serde_json::json!({"action": "list"}), ctx.clone());
    assert!(out.contains(&id), "list should contain {id}: {out}");
    assert!(out.contains("running"), "{id} should be running: {out}");

    // logs show its output.
    let out = call(
        "service",
        serde_json::json!({"action": "logs", "id": id}),
        ctx.clone(),
    );
    assert!(out.contains("ready"), "log should have 'ready': {out}");

    // stop kills it.
    let out = call(
        "service",
        serde_json::json!({"action": "stop", "id": id}),
        ctx.clone(),
    );
    assert!(out.contains("stopped"), "stop should report: {out}");

    // gone from the registry.
    let out = call("service", serde_json::json!({"action": "list"}), ctx);
    assert!(!out.contains(&id), "{id} should be gone: {out}");
}

/// A listener launched `persistent` must be reachable FROM THE HOST —
/// that is the whole point of the audit's server→shared-network mapping
/// (a private netns would strand it on a loopback nobody else can see).
#[test]
fn persistent_server_reachable_from_host() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "hello-host").unwrap();
    let ctx = Arc::new(ctx_with_sandbox(dir.path()));

    let out = call(
        "bash",
        serde_json::json!({
            "command": "python3 -m http.server 18723 --bind 127.0.0.1",
            "persistent": true
        }),
        ctx.clone(),
    );
    assert!(out.contains("svc-"), "spawn failed: {out}");
    let id = out.split_whitespace()
        .find(|t| t.starts_with("id="))
        .unwrap()
        .trim_start_matches("id=")
        .to_string();

    // Give it a moment, then hit it from the host (this test process).
    let mut got = String::new();
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Ok(mut s) = std::net::TcpStream::connect("127.0.0.1:18723") {
            use std::io::{Read, Write};
            let _ = s.write_all(b"GET / HTTP/1.0\r\n\r\n");
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            got = buf;
            if got.contains("hello-host") {
                break;
            }
        }
    }
    assert!(
        got.contains("hello-host"),
        "host should reach the service — got: {got}"
    );

    let out = call(
        "service",
        serde_json::json!({"action": "stop", "id": id}),
        ctx,
    );
    assert!(out.contains("stopped"), "{out}");
}

/// The master switch: `enabled:false` routes process tools to the loud
/// `none` backend — same ToolCtx, live toggle, no session restart. The host
/// installs the switchable wrapper; the context itself reads nothing global.
#[test]
fn sandbox_disabled_routes_to_none() {
    let dir = tempfile::tempdir().unwrap();
    // Local, injected switch source: the test never touches the process-global
    // overrides, so it can run in parallel with other sandbox tests.
    let switch = Arc::new(std::sync::RwLock::new(SandboxLimits::default()));
    let (detected, _loud) = agent_sandbox::detect_backend();
    let mut ctx = ToolCtx::new().with_workspace(dir.path());
    let src = switch.clone();
    ctx.install_sandbox(Arc::new(SwitchableSandbox::with_limits(
        Arc::from(detected),
        move || src.read().unwrap().clone(),
    )));
    let ctx = Arc::new(ctx);
    let on = ctx.sandbox().unwrap().id();
    assert_ne!(on, "none-simulated");

    switch.write().unwrap().enabled = Some(false);
    assert_eq!(ctx.sandbox().unwrap().id(), "none");

    // off → command runs unsandboxed (HOME is visible again on `none`)
    let out = call(
        "bash",
        serde_json::json!({"command": "echo $HOME | head -c 5"}),
        ctx.clone(),
    );
    assert!(out.contains("/home") || out.contains("/root"), "{out}");

    *switch.write().unwrap() = SandboxLimits::default();
    assert_eq!(ctx.sandbox().unwrap().id(), on);
}
