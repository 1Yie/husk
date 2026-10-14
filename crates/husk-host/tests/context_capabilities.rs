//! Optional backend installation and inheritance through the generic context.

use std::sync::Arc;

use agent_capabilities::{DesktopCtx, SandboxCtx, WorkspaceCtx};
use agent_kernel::tools::ToolCtx;

struct Marker;

#[tokio::test]
async fn bare_context_refuses_local_tools_without_a_workspace() {
    let ctx = Arc::new(ToolCtx::new());
    assert!(ctx.root().is_err());
    let registry = husk_host::session::base_registries().0;
    for (name, args) in [
        ("list_dir", serde_json::json!({"path": "."})),
        ("smart_read", serde_json::json!({"path": "missing.txt"})),
        (
            "bash",
            serde_json::json!({"command": "echo should-not-run"}),
        ),
        ("serena", serde_json::json!({"tool": "find_symbol"})),
    ] {
        let error = registry
            .dispatch(name, args, ctx.clone())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("workspace is not available"),
            "{name}: {error}"
        );
    }
}

#[test]
fn installed_workspace_is_shared_with_scoped_and_child_contexts() {
    let dir = tempfile::tempdir().unwrap();
    let mut parent = ToolCtx::new().with_workspace(dir.path());
    parent.ext.insert(Arc::new(Marker));
    let scoped = parent.with_cancel(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let child = ToolCtx {
        ext: parent.ext.inheritable(),
        ..ToolCtx::new()
    };
    assert!(std::ptr::eq(parent.root().unwrap(), scoped.root().unwrap()));
    assert!(std::ptr::eq(parent.root().unwrap(), child.root().unwrap()));
    assert!(child.require::<Marker>("contract").is_err());
}

#[test]
fn a_context_without_a_desktop_refuses_and_never_probes() {
    // The kernel no longer probes hardware in `new`: a host with no display
    // (an SSH worker, CI) gets a refusal with a reason, not a panic.
    let ctx = ToolCtx::new().with_workspace(std::env::temp_dir());
    let err = ctx.desktop().err().expect("no desktop was installed");
    assert!(err.to_string().contains("desktop control is not available"));
}

#[test]
fn child_inherits_only_the_desktop_not_the_controllers() {
    let mut parent = ToolCtx::new()
        .with_workspace(std::env::temp_dir())
        .with_desktop(Arc::new(agent_computer::NoneBackend::new("probe")));
    parent.ext.insert(Arc::new(Marker));
    let mut child = ToolCtx::new().with_workspace(std::env::temp_dir());
    child.ext = parent.ext.inheritable();
    // The same backend instance, so a child screenshot steers the parent's
    // coordinate mapping exactly as the parent's own would.
    assert!(Arc::ptr_eq(
        &parent.desktop().unwrap(),
        &child.desktop().unwrap()
    ));
    // And nothing else was copied wholesale: the parent's slots (including
    // the contract controllers a host installs) do not leak into the child.
    assert!(child.require::<Marker>("contract").is_err());
}

#[test]
fn a_context_without_a_sandbox_refuses_shell_execution() {
    // An SSH worker installs no local sandbox. `bash` must refuse, not run
    // unisolated because nobody said otherwise.
    let ctx = ToolCtx::new().with_workspace(std::env::temp_dir());
    let err = ctx.sandbox().err().expect("no sandbox was installed");
    assert!(err.to_string().contains("shell execution is not available"));
}

#[test]
fn an_installed_none_backend_still_runs_and_says_so() {
    // A deliberately disabled sandbox is a different state from an absent
    // one: the backend is present, reports `none`, and commands still run.
    let ctx = ToolCtx::new()
        .with_workspace(std::env::temp_dir())
        .with_sandbox(Arc::new(agent_sandbox::NoneBackend));
    assert_eq!(ctx.sandbox().unwrap().id(), "none");
}

#[test]
fn child_inherits_the_sandbox_and_its_limits() {
    let mut parent = ToolCtx::new()
        .with_workspace(std::env::temp_dir())
        .with_sandbox(Arc::new(agent_sandbox::NoneBackend))
        .with_sandbox_limits(agent_sandbox::SandboxLimits {
            network: Some(false),
            ..Default::default()
        });
    // A slot the host installed (the contract controllers live in
    // agent-coding now — a marker stands in for the scoping check).
    parent.ext.insert(Arc::new(Marker));
    let mut child = ToolCtx::new().with_workspace(std::env::temp_dir());
    child.ext = parent.ext.inheritable();
    // The child's bash must obey the host's forced network policy.
    assert_eq!(child.sandbox_limits().network, Some(false));
    assert!(child.sandbox().is_ok());
    // And nothing else was copied wholesale: the parent's slots (including
    // the contract controllers a host installs) do not leak into the child.
    assert!(child.require::<Marker>("contract").is_err());
}

#[test]
fn resolve_new_file_in_missing_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ToolCtx::new().with_workspace(tmp.path());
    let got = ctx.resolve("brand-new/nested/out.docx").unwrap();
    assert_eq!(
        got,
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("brand-new/nested/out.docx")
    );
}

/// Paths that escape the workspace still fail even through the walk.
#[test]
fn resolve_escape_still_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ToolCtx::new().with_workspace(tmp.path());
    assert!(ctx.resolve("../../outside.txt").is_err());
}

/// A dangling symlink whose target lies outside the workspace must not
/// resolve: the later write would follow the link out of the sandbox.
#[cfg(unix)]
#[test]
fn resolve_rejects_dangling_symlink_to_outside() {
    let ws = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("not-yet.txt");
    std::os::unix::fs::symlink(&target, ws.path().join("link.txt")).unwrap();
    let ctx = ToolCtx::new().with_workspace(ws.path());
    assert!(ctx.resolve("link.txt").is_err());
    assert!(ctx.resolve("link.txt/child").is_err());
    assert!(!target.exists());
}
