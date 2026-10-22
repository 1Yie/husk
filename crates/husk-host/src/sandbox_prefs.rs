//! `sandbox_prefs` — process-wide sandbox overrides from the settings UI.
//!
//! Deliberately a process global: the settings page and the CLI flags change
//! the limits for every live session at once, so one process has exactly one
//! source. A host that needs per-session limits builds its sandbox with
//! [`SwitchableSandbox::with_limits`] instead of [`SwitchableSandbox::new`] —
//! that seam is the injection point, and the one place to widen if two
//! products ever share a process.
//!
//! Process tools (`bash`, `test_runner`) fold these into the per-run
//! `SandboxConfig` AFTER the audit plan fills its defaults — `None` fields
//! leave the plan's values untouched.

use std::path::PathBuf;
use std::sync::RwLock;

use agent_sandbox::SandboxConfig;

// The override type and its folding rules live in `agent-sandbox`; re-exported
// so existing `sandbox_prefs::SandboxLimits` paths keep working.
pub use agent_sandbox::{apply_limits, network_from_label, SandboxLimits};

static LIMITS: RwLock<SandboxLimits> = RwLock::new(SandboxLimits {
    enabled: None,
    network: None,
    max_memory_mb: None,
    max_processes: None,
    allowed_domains: Vec::new(),
    denied_domains: Vec::new(),
    extra_ro_mounts: Vec::new(),
    extra_rw_mounts: Vec::new(),
});

/// Current overrides — a poisoned read yields defaults rather than
/// panicking inside a tool call.
pub fn current() -> SandboxLimits {
    LIMITS.read().map(|l| l.clone()).unwrap_or_default()
}

/// Replace the overrides — seeded from persisted prefs at kernel boot and
/// updated by `set_default_prefs`.
pub fn set(limits: SandboxLimits) {
    if let Ok(mut w) = LIMITS.write() {
        *w = limits;
    }
}

/// Fold the overrides into a per-run `SandboxConfig` after the audit plan
/// filled its defaults.
pub fn apply_to(cfg: &mut SandboxConfig) {
    apply_limits(cfg, &current());
}

/// Current and historical product configuration paths. The sandbox library
/// keeps generic secret defaults; the host supplies its own private state.
pub fn protected_paths(paths: &agent_paths::AppPaths) -> Vec<PathBuf> {
    let mut denied = Vec::new();
    if !paths.config_dir.as_os_str().is_empty() {
        denied.push(paths.config_dir.clone());
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for name in [".config/husk", ".config/agent-rs"] {
            let path = home.join(name);
            if !denied.contains(&path) {
                denied.push(path);
            }
        }
    }
    denied
}

/// The host's sandbox, honouring the live master switch.
///
/// The switch can flip mid-session (the settings page), so the choice is made
/// per call, not once at spawn: `enabled` reads the current overrides and
/// forwards to the real backend, or to `none` when isolation is off. `id` and
/// `tier` follow the same choice, so the UI never names a backend that is not
/// the one actually running the command.
pub struct SwitchableSandbox {
    real: std::sync::Arc<dyn agent_sandbox::SandboxBackend>,
    limits: std::sync::Arc<dyn Fn() -> SandboxLimits + Send + Sync>,
    deny_read: Vec<PathBuf>,
}

impl SwitchableSandbox {
    /// Reads the process-wide overrides on every call (product path).
    pub fn new(real: std::sync::Arc<dyn agent_sandbox::SandboxBackend>) -> Self {
        Self::with_limits(real, current)
    }

    /// Reads the switch from `limits` on every call — lets a host or test
    /// supply its own source instead of the process-global overrides.
    pub fn with_limits(
        real: std::sync::Arc<dyn agent_sandbox::SandboxBackend>,
        limits: impl Fn() -> SandboxLimits + Send + Sync + 'static,
    ) -> Self {
        Self {
            real,
            limits: std::sync::Arc::new(limits),
            deny_read: Vec::new(),
        }
    }

    /// Protect host-owned paths on both short commands and persistent spawns.
    pub fn with_deny_read(mut self, paths: Vec<PathBuf>) -> Self {
        self.deny_read = paths;
        self
    }

    fn config(&self, cfg: &SandboxConfig) -> SandboxConfig {
        let mut cfg = cfg.clone();
        cfg.deny_read.extend(self.deny_read.iter().cloned());
        cfg
    }

    fn active(&self) -> &dyn agent_sandbox::SandboxBackend {
        static NONE: agent_sandbox::NoneBackend = agent_sandbox::NoneBackend;
        if (self.limits)().enabled() {
            &*self.real
        } else {
            &NONE
        }
    }
}

#[async_trait::async_trait]
impl agent_sandbox::SandboxBackend for SwitchableSandbox {
    fn id(&self) -> &'static str {
        self.active().id()
    }

    fn tier(&self) -> agent_sandbox::SandboxTier {
        self.active().tier()
    }

    async fn run_command(
        &self,
        cmd: &str,
        args: &[&str],
        cfg: &SandboxConfig,
    ) -> anyhow::Result<agent_sandbox::CommandOutput> {
        let cfg = self.config(cfg);
        self.active().run_command(cmd, args, &cfg).await
    }

    fn wrap_spawn(
        &self,
        cfg: &SandboxConfig,
        cmd: &str,
        args: &[&str],
    ) -> Option<tokio::process::Command> {
        self.active().wrap_spawn(&self.config(cfg), cmd, args)
    }
}
