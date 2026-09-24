//! Shared audit → sandbox-config derivation.
//!
//! Both `bash` and `smart_test_runner` run a shell command through
//! `ctx.sandbox`; the mount/network/resource policy must be identical between
//! them (a test run is not a lesser command), so the derivation lives here
//! instead of in either tool.

use std::path::Path;
use std::time::Duration;

/// Audit verdict → sandbox configuration.
///
/// Policy → SandboxPlan → SandboxConfig: the plan derives network/fs limits
/// from the audit's capabilities (Network/PackageInstall flips `allow_network`;
/// High+ risk forces a CoW snapshot), and the L2 backend consumes the config.
/// Split out from the tools' `exec` so the mount policy is assertable without
/// spawning anything.
pub(crate) fn sandbox_config(
    verdict: &agent_sandbox::AuditResult,
    workspace_root: &Path,
    timeout: Duration,
) -> agent_sandbox::SandboxConfig {
    let plan = agent_sandbox::plan::SandboxPlan::from_audit(
        verdict,
        workspace_root.to_path_buf(),
        std::env::temp_dir().join(format!("husk-{:x}", std::process::id())),
    );
    let mut cfg = agent_sandbox::SandboxConfig {
        workspace_dir: workspace_root.to_path_buf(),
        allow_network: !matches!(plan.network, agent_sandbox::plan::NetworkPolicy::Deny),
        max_memory_mb: plan.processes.max_memory_mb,
        max_processes: plan.processes.max_processes,
        timeout_secs: timeout.as_secs(),
        snapshot: plan.snapshot,
        environment: plan.environment.clone(),
        // The agent's own user-level skills, read-only. They live outside the
        // workspace, so the mount whitelist would otherwise hide them and
        // `bash`/`read` could not open a skill's sibling files. Read-only on
        // purpose: a skill must never be able to rewrite its own instructions.
        extra_ro_mounts: crate::skills::scanner::global_skill_roots(),
        ..Default::default()
    };
    // User overrides from the settings UI fold in last — a forced network
    // policy / memory / process cap beats the audit plan's defaults.
    crate::sandbox_prefs::apply_to(&mut cfg);
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    /// User-level skills sit outside the workspace, so the mount whitelist has
    /// to be widened for them, or a sandboxed command cannot read the files a
    /// skill points at. Read-only: skills are not ours to rewrite.
    #[test]
    fn sandbox_config_mounts_global_skills_read_only() {
        let root = Path::new("/tmp/ws");
        let verdict = agent_sandbox::audit_command_scoped("ls", Some(root));
        let cfg = sandbox_config(&verdict, root, Duration::from_secs(30));

        assert_eq!(
            cfg.extra_ro_mounts,
            crate::skills::scanner::global_skill_roots()
        );
        assert!(
            !cfg.extra_ro_mounts.is_empty(),
            "no $HOME would hide every skill"
        );
        assert!(
            cfg.extra_rw_mounts.is_empty(),
            "skills must not be writable"
        );
        assert_eq!(cfg.workspace_dir, root);
    }
}
