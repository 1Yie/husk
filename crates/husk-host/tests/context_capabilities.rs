//! Optional backend installation and inheritance through the generic context.

use std::sync::Arc;

use agent_capabilities::{DesktopCtx, SandboxCtx};
use agent_kernel::tools::ToolCtx;

struct Marker;

#[test]
fn a_context_without_a_desktop_refuses_and_never_probes() {
    // The kernel no longer probes hardware in `new`: a host with no display
    // (an SSH worker, CI) gets a refusal with a reason, not a panic.
    let ctx = ToolCtx::new(std::env::temp_dir());
    let err = ctx.desktop().err().expect("no desktop was installed");
    assert!(err.to_string().contains("desktop control is not available"));
}

#[test]
fn child_inherits_only_the_desktop_not_the_controllers() {
    let mut parent = ToolCtx::new(std::env::temp_dir())
        .with_desktop(Arc::new(agent_computer::NoneBackend::new("probe")));
    parent.ext.insert(Arc::new(Marker));
    let mut child = ToolCtx::new(std::env::temp_dir());
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
    let ctx = ToolCtx::new(std::env::temp_dir());
    let err = ctx.sandbox().err().expect("no sandbox was installed");
    assert!(err.to_string().contains("shell execution is not available"));
}

#[test]
fn an_installed_none_backend_still_runs_and_says_so() {
    // A deliberately disabled sandbox is a different state from an absent
    // one: the backend is present, reports `none`, and commands still run.
    let ctx = ToolCtx::new(std::env::temp_dir()).with_sandbox(Arc::new(agent_sandbox::NoneBackend));
    assert_eq!(ctx.sandbox().unwrap().id(), "none");
}

#[test]
fn child_inherits_the_sandbox_and_its_limits() {
    let mut parent = ToolCtx::new(std::env::temp_dir())
        .with_sandbox(Arc::new(agent_sandbox::NoneBackend))
        .with_sandbox_limits(agent_sandbox::SandboxLimits {
            network: Some(false),
            ..Default::default()
        });
    // A slot the host installed (the contract controllers live in
    // agent-coding now — a marker stands in for the scoping check).
    parent.ext.insert(Arc::new(Marker));
    let mut child = ToolCtx::new(std::env::temp_dir());
    child.ext = parent.ext.inheritable();
    // The child's bash must obey the host's forced network policy.
    assert_eq!(child.sandbox_limits().network, Some(false));
    assert!(child.sandbox().is_ok());
    // And nothing else was copied wholesale: the parent's slots (including
    // the contract controllers a host installs) do not leak into the child.
    assert!(child.require::<Marker>("contract").is_err());
}
